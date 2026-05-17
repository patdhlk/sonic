//! Per-iface dispatcher tasks. `ARCH_0060`, `ARCH_0061`, `ARCH_0062`.
//!
//! Each owned CAN interface runs as one tokio task that owns its
//! `CanInterfaceLike` driver instance. The task loop:
//!
//! 1. Recompute and re-apply the per-iface filter when the registry's
//!    Inbound channel count for this iface has changed since last
//!    apply (`REQ_0522`, `REQ_0523`).
//! 2. Drain outbound bindings for this iface — build a `CanFrame` per
//!    drained envelope and `send_classical` / `send_fd` via the driver
//!    (`ARCH_0060`, `REQ_0513`).
//! 3. Await one inbound frame with a TX tick timeout. On data, demux
//!    to every reader binding whose routing matches under
//!    `filter::matches` (`ARCH_0061`, `REQ_0514`, `REQ_0524`). On
//!    error, classify and drive the health state machine; on bus-off,
//!    consult [`sonic_connector_core::ReconnectPolicy`] and reopen
//!    (`ARCH_0062`, `REQ_0533`, `REQ_0534`).
//!
//! The task exits cleanly when its `stop` signal flips to `true`.
//!
//! The trait-object wrappers [`IoxOutboundDrain`] / [`IoxInboundPublish`]
//! erase the channel's user-type `T` and codec `C` from the
//! [`ChannelBinding`] so the registry stores heterogeneous channels
//! in a single `Vec`. The codec never runs on the dispatcher path —
//! payload decoding stays in
//! [`sonic_connector_transport_iox::ChannelReader::try_recv`].

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sonic_connector_core::{ConnectorError, ReconnectPolicy};
use sonic_connector_transport_iox::{RawChannelReader, RawChannelWriter};

use crate::driver::{
    CanData, CanErrorKind, CanFrame, CanIfaceState, CanInterfaceLike, CanIoError, MAX_CAN_PAYLOAD,
};
use crate::filter::{self, PerIfaceFilter};
use crate::health::{CanHealthMonitor, IfaceHealthKind};
use crate::registry::{
    ChannelBinding, ChannelRegistry, Direction, InboundPublish, OutboundDrain, RegisteredChannel,
};
use crate::routing::{CanFrameKind, CanIface};

/// Default per-iteration TX drain tick. Sets the upper bound on
/// outbound latency when the bus is otherwise quiet.
pub const DEFAULT_TX_TICK: Duration = Duration::from_millis(1);

/// Trait-object wrapper around a gateway-side [`RawChannelReader`].
///
/// Implements [`OutboundDrain`]. The caller-supplied `dest` slice
/// doubles as the iceoryx2 receive buffer.
pub struct IoxOutboundDrain<const N: usize> {
    reader: RawChannelReader<N>,
}

impl<const N: usize> IoxOutboundDrain<N> {
    /// Construct a drain wrapping `reader`.
    #[must_use]
    pub const fn new(reader: RawChannelReader<N>) -> Self {
        Self { reader }
    }
}

impl<const N: usize> OutboundDrain for IoxOutboundDrain<N> {
    fn drain_into(&self, dest: &mut [u8]) -> Result<Option<usize>, ConnectorError> {
        let Some(sample) = self.reader.try_recv_into(dest)? else {
            return Ok(None);
        };
        Ok(Some(sample.payload_len))
    }
}

/// Trait-object wrapper around a gateway-side [`RawChannelWriter`] —
/// implements [`InboundPublish`].
pub struct IoxInboundPublish<const N: usize> {
    writer: RawChannelWriter<N>,
}

impl<const N: usize> IoxInboundPublish<N> {
    /// Construct a publisher wrapping `writer`.
    #[must_use]
    pub const fn new(writer: RawChannelWriter<N>) -> Self {
        Self { writer }
    }
}

impl<const N: usize> InboundPublish for IoxInboundPublish<N> {
    fn publish_bytes(&self, bytes: &[u8]) -> Result<(), ConnectorError> {
        self.writer.send_raw_bytes(bytes, [0u8; 32]).map(|_| ())
    }
}

/// External commands the dispatcher accepts. Used by the connector
/// to signal "filter recomputed" or "stop".
#[derive(Debug)]
pub enum DispatcherCommand {
    /// Caller registered or dropped a channel — re-apply the filter
    /// on the next loop iteration.
    FilterDirty,
}

/// Per-iface task entry point. Spawned by
/// [`crate::CanConnector`]'s `register_with` impl for every iface in
/// [`crate::CanConnectorOptions::ifaces`].
///
/// # Errors
///
/// Returns the first unrecoverable error from the driver. Recoverable
/// errors (error-passive, bus-off) are absorbed by the health state
/// machine and do not return.
pub async fn dispatcher_loop<I>(
    iface: CanIface,
    mut driver: I,
    registry: Arc<Mutex<ChannelRegistry>>,
    health: Arc<CanHealthMonitor>,
    mut reconnect_policy: Box<dyn ReconnectPolicy>,
    stop: Arc<AtomicBool>,
    tx_tick: Duration,
) -> Result<(), ConnectorError>
where
    I: CanInterfaceLike,
{
    let mut last_filter: PerIfaceFilter = PerIfaceFilter::default();
    let mut last_inbound_count: usize = 0;

    // Bring-up: initial filter apply + transition to Up.
    let initial = compile_iface_filter(&registry, &iface);
    if initial != last_filter {
        if let Err(e) = driver.apply_filter(initial.as_slice()) {
            let _ = health.set_iface(iface, IfaceHealthKind::Down);
            return Err(ConnectorError::stack(IoErr(format!("apply_filter: {e}"))));
        }
        last_filter = initial;
    }
    let _ = health.set_iface(iface, IfaceHealthKind::Up);

    let mut tx_scratch = [0u8; MAX_CAN_PAYLOAD];

    while !stop.load(Ordering::Acquire) {
        // Filter recompute if inbound count changed.
        let current_count = inbound_count(&registry, &iface);
        if current_count != last_inbound_count {
            let recompiled = compile_iface_filter(&registry, &iface);
            if recompiled != last_filter {
                if let Err(e) = driver.apply_filter(recompiled.as_slice()) {
                    return Err(ConnectorError::stack(IoErr(format!(
                        "filter recompile apply: {e}"
                    ))));
                }
                last_filter = recompiled;
            }
            last_inbound_count = current_count;
        }

        // TX drain: pull every pending outbound envelope for this iface
        // and send via the driver.
        drain_outbound_once(&iface, &registry, &mut driver, &mut tx_scratch).await?;

        // RX: await one frame or the TX tick deadline. The select block
        // ends before any further `&mut driver` calls so the recv future
        // is dropped and `driver` is free again.
        let result: Option<Result<CanFrame, CanIoError>> = {
            let recv = driver.recv();
            tokio::pin!(recv);
            tokio::select! {
                biased;
                _ = tokio::time::sleep(tx_tick) => None,
                res = &mut recv => Some(res),
            }
        };

        match result {
            None => continue,
            Some(Ok(frame)) => {
                handle_inbound_frame(
                    &iface,
                    frame,
                    &registry,
                    &health,
                    &mut driver,
                    &mut *reconnect_policy,
                )
                .await?;
            }
            Some(Err(CanIoError::Closed)) => {
                let _ = health.set_iface(iface, IfaceHealthKind::Down);
                if !reconnect_until_open(&iface, &mut driver, &mut *reconnect_policy, &stop).await {
                    return Ok(());
                }
                let recompiled = compile_iface_filter(&registry, &iface);
                let _ = driver.apply_filter(recompiled.as_slice());
                last_filter = recompiled;
                let _ = health.set_iface(iface, IfaceHealthKind::Connecting);
                let _ = health.set_iface(iface, IfaceHealthKind::Up);
            }
            Some(Err(e)) => return Err(ConnectorError::stack(IoErr(format!("recv: {e}")))),
        }
    }
    Ok(())
}

/// Per-iteration outcome surfaced from [`dispatch_one_iteration`].
/// Test harnesses use this to assert TX/RX progress without a
/// long-running background task.
#[derive(Debug, Default)]
pub struct IterationOutcome {
    /// Number of outbound frames sent on the driver this iteration.
    pub tx_sent: usize,
    /// Number of inbound data frames demuxed to readers this
    /// iteration (each matching reader counted once).
    pub inbound_publishes: usize,
    /// Error-frame discriminant observed, if any.
    pub error_kind: Option<CanErrorKind>,
    /// `true` when the inbound recv returned `Closed` and the
    /// dispatcher took the reconnect path.
    pub reconnected: bool,
}

/// Drive one full RX + TX iteration synchronously. Test harnesses
/// call this to step through the dispatcher state machine without
/// spawning [`dispatcher_loop`] on a runtime. Production code uses
/// [`dispatcher_loop`] instead.
///
/// Behaviour mirrors one iteration of [`dispatcher_loop`]: filter
/// recompute on registry change, TX drain, then one RX await with
/// the given timeout. Returns when the timeout elapses or one frame
/// is processed.
///
/// # Errors
///
/// Propagates driver / filter / publish errors verbatim.
pub async fn dispatch_one_iteration<I>(
    iface: &CanIface,
    driver: &mut I,
    registry: &Arc<Mutex<ChannelRegistry>>,
    health: &Arc<CanHealthMonitor>,
    reconnect_policy: &mut dyn ReconnectPolicy,
    recv_timeout: Duration,
) -> Result<IterationOutcome, ConnectorError>
where
    I: CanInterfaceLike,
{
    let mut outcome = IterationOutcome::default();

    // Filter recompute pass.
    let recompiled = compile_iface_filter(registry, iface);
    let last = inbound_count(registry, iface);
    if !recompiled.is_empty() || last > 0 {
        driver
            .apply_filter(recompiled.as_slice())
            .map_err(|e| ConnectorError::stack(IoErr(format!("apply_filter: {e}"))))?;
    }

    // TX drain.
    let mut tx_scratch = [0u8; MAX_CAN_PAYLOAD];
    let jobs = collect_outbound_jobs(iface, registry)?;
    for data in &jobs {
        match data.kind {
            CanFrameKind::Classical => {
                driver
                    .send_classical(data)
                    .await
                    .map_err(|e| ConnectorError::stack(IoErr(format!("send_classical: {e}"))))?;
            }
            CanFrameKind::Fd => {
                driver
                    .send_fd(data)
                    .await
                    .map_err(|e| ConnectorError::stack(IoErr(format!("send_fd: {e}"))))?;
            }
        }
        outcome.tx_sent += 1;
    }
    let _ = &mut tx_scratch;

    // RX poll with timeout.
    let result: Option<Result<CanFrame, CanIoError>> = {
        let recv = driver.recv();
        tokio::pin!(recv);
        tokio::select! {
            biased;
            _ = tokio::time::sleep(recv_timeout) => None,
            res = &mut recv => Some(res),
        }
    };

    match result {
        None => {}
        Some(Ok(CanFrame::Data(d))) => {
            outcome.inbound_publishes = demux_inbound_counted(iface, &d, registry)?;
        }
        Some(Ok(CanFrame::Error(kind))) => {
            outcome.error_kind = Some(kind);
            classify_error(iface, kind, health, driver, reconnect_policy).await?;
        }
        Some(Ok(CanFrame::Remote { .. })) => {}
        Some(Err(CanIoError::Closed)) => {
            outcome.reconnected = true;
            let _ = health.set_iface(*iface, IfaceHealthKind::Down);
            // Try one reopen with the policy's next delay.
            let delay = reconnect_policy.next_delay();
            tokio::time::sleep(delay).await;
            if driver.reopen().await.is_ok() {
                let recompiled = compile_iface_filter(registry, iface);
                let _ = driver.apply_filter(recompiled.as_slice());
                let _ = health.set_iface(*iface, IfaceHealthKind::Connecting);
                let _ = health.set_iface(*iface, IfaceHealthKind::Up);
            }
        }
        Some(Err(e)) => return Err(ConnectorError::stack(IoErr(format!("recv: {e}")))),
    }

    Ok(outcome)
}

#[allow(clippy::significant_drop_tightening)]
fn demux_inbound_counted(
    iface: &CanIface,
    frame: &CanData,
    registry: &Mutex<ChannelRegistry>,
) -> Result<usize, ConnectorError> {
    let guard = registry.lock().expect("registry mutex not poisoned");
    let mut count = 0usize;
    for entry in guard.iter_iface_direction(iface, Direction::Inbound) {
        let RegisteredChannel {
            routing, binding, ..
        } = entry;
        if !filter::matches(routing, frame) {
            continue;
        }
        if let ChannelBinding::Inbound(publish) = binding {
            publish.publish_bytes(frame.payload())?;
            count += 1;
        }
    }
    Ok(count)
}

async fn handle_inbound_frame<I>(
    iface: &CanIface,
    frame: CanFrame,
    registry: &Mutex<ChannelRegistry>,
    health: &Arc<CanHealthMonitor>,
    driver: &mut I,
    reconnect_policy: &mut dyn ReconnectPolicy,
) -> Result<(), ConnectorError>
where
    I: CanInterfaceLike,
{
    match frame {
        CanFrame::Data(d) => {
            demux_inbound(iface, &d, registry)?;
            Ok(())
        }
        CanFrame::Error(kind) => {
            classify_error(iface, kind, health, driver, reconnect_policy).await
        }
        CanFrame::Remote { .. } => {
            // Remote frames pass silently in layer-1; future layer-2
            // can either drop or surface them per configuration.
            Ok(())
        }
    }
}

#[allow(clippy::significant_drop_tightening)] // guard intentionally held
fn demux_inbound(
    iface: &CanIface,
    frame: &CanData,
    registry: &Mutex<ChannelRegistry>,
) -> Result<(), ConnectorError> {
    let guard = registry.lock().expect("registry mutex not poisoned");
    for entry in guard.iter_iface_direction(iface, Direction::Inbound) {
        let RegisteredChannel {
            routing, binding, ..
        } = entry;
        if !filter::matches(routing, frame) {
            continue;
        }
        if let ChannelBinding::Inbound(publish) = binding {
            publish.publish_bytes(frame.payload())?;
        }
    }
    Ok(())
}

async fn classify_error<I>(
    iface: &CanIface,
    kind: CanErrorKind,
    health: &Arc<CanHealthMonitor>,
    driver: &mut I,
    reconnect_policy: &mut dyn ReconnectPolicy,
) -> Result<(), ConnectorError>
where
    I: CanInterfaceLike,
{
    match kind {
        CanErrorKind::Warning | CanErrorKind::Passive => {
            let _ = health.set_iface(*iface, IfaceHealthKind::Degraded);
            Ok(())
        }
        CanErrorKind::BusOff => {
            let _ = health.set_iface(*iface, IfaceHealthKind::Down);
            let delay = reconnect_policy.next_delay();
            tokio::time::sleep(delay).await;
            match driver.reopen().await {
                Ok(()) => {
                    let _ = health.set_iface(*iface, IfaceHealthKind::Connecting);
                    let _ = health.set_iface(*iface, IfaceHealthKind::Up);
                    Ok(())
                }
                Err(e) => Err(ConnectorError::stack(IoErr(format!("reopen: {e}")))),
            }
        }
        CanErrorKind::ArbitrationLost | CanErrorKind::Other => Ok(()),
    }
}

async fn reconnect_until_open<I>(
    iface: &CanIface,
    driver: &mut I,
    reconnect_policy: &mut dyn ReconnectPolicy,
    stop: &Arc<AtomicBool>,
) -> bool
where
    I: CanInterfaceLike,
{
    let _ = iface;
    while !stop.load(Ordering::Acquire) {
        let delay = reconnect_policy.next_delay();
        tokio::time::sleep(delay).await;
        if driver.reopen().await.is_ok() && matches!(driver.state(), CanIfaceState::Active) {
            return true;
        }
    }
    false
}

async fn drain_outbound_once<I>(
    iface: &CanIface,
    registry: &Mutex<ChannelRegistry>,
    driver: &mut I,
    _tx_scratch: &mut [u8; MAX_CAN_PAYLOAD],
) -> Result<(), ConnectorError>
where
    I: CanInterfaceLike,
{
    // Pull every pending envelope into an owned Vec while holding the
    // registry lock, then release the lock before awaiting any send.
    // Bounded by per-iface outbound channels × pending envelopes per
    // channel — typical layer-1 case is a handful per tick.
    let jobs: Vec<CanData> = collect_outbound_jobs(iface, registry)?;
    for data in &jobs {
        match data.kind {
            CanFrameKind::Classical => {
                driver
                    .send_classical(data)
                    .await
                    .map_err(|e| ConnectorError::stack(IoErr(format!("send_classical: {e}"))))?;
            }
            CanFrameKind::Fd => {
                driver
                    .send_fd(data)
                    .await
                    .map_err(|e| ConnectorError::stack(IoErr(format!("send_fd: {e}"))))?;
            }
        }
    }
    Ok(())
}

#[allow(clippy::significant_drop_tightening)] // guard intentionally held
fn collect_outbound_jobs(
    iface: &CanIface,
    registry: &Mutex<ChannelRegistry>,
) -> Result<Vec<CanData>, ConnectorError> {
    let guard = registry.lock().expect("registry mutex not poisoned");
    let mut jobs: Vec<CanData> = Vec::new();
    let mut buf = [0u8; MAX_CAN_PAYLOAD];
    for entry in guard.iter_iface_direction(iface, Direction::Outbound) {
        let RegisteredChannel {
            routing, binding, ..
        } = entry;
        let ChannelBinding::Outbound(drain) = binding else {
            continue;
        };
        loop {
            let Some(written) = drain.drain_into(&mut buf)? else {
                break;
            };
            let data = CanData::new(
                routing.can_id,
                routing.kind,
                routing.fd_flags,
                &buf[..written],
            )
            .map_err(|e| ConnectorError::stack(IoErr(format!("build frame: {e}"))))?;
            jobs.push(data);
        }
    }
    Ok(jobs)
}

#[allow(clippy::significant_drop_tightening)]
fn compile_iface_filter(registry: &Mutex<ChannelRegistry>, iface: &CanIface) -> PerIfaceFilter {
    let guard = registry.lock().expect("registry mutex not poisoned");
    let routings = guard
        .iter_iface_direction(iface, Direction::Inbound)
        .map(|entry| entry.routing);
    filter::compile(routings)
}

#[allow(clippy::significant_drop_tightening)]
fn inbound_count(registry: &Mutex<ChannelRegistry>, iface: &CanIface) -> usize {
    let guard = registry.lock().expect("registry mutex not poisoned");
    guard
        .iter_iface_direction(iface, Direction::Inbound)
        .count()
}

#[derive(Debug)]
struct IoErr(String);

impl core::fmt::Display for IoErr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "can dispatcher: {}", self.0)
    }
}

impl std::error::Error for IoErr {}
