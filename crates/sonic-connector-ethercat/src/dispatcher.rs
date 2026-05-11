//! Gateway-side dispatcher. `REQ_0326`, `REQ_0327`, `REQ_0328`.
//!
//! Composes [`pdi::write_routing`] / [`pdi::read_routing`] +
//! [`ChannelRegistry`] + the iceoryx2 raw pub/sub ports into the
//! actual byte-shovel between iceoryx2 and the EtherCAT PDI.
//!
//! Two surfaces:
//!
//! * [`dispatch_one_cycle`] — synchronous, single-iteration form used
//!   by integration tests to drive deterministic round-trips
//!   (`TEST_0220` / `TEST_0221` / `TEST_0222`). The test owns the
//!   [`CycleRunner`] and feeds the dispatcher cycle-by-cycle.
//! * [`dispatcher_loop`] — long-running `async fn` spawned by
//!   `EthercatConnector::register_with` (`REQ_0321`). The loop calls
//!   `dispatch_one_cycle` on the gateway's tokio runtime and sleeps
//!   until the next cycle deadline.
//!
//! The trait-object wrappers [`IoxOutboundDrain`] / [`IoxInboundPublish`]
//! erase the channel's user-type `T` and codec `C` from the
//! [`ChannelBinding`] so the registry stores heterogeneous channels
//! in a single `Vec`. The codec never runs on the dispatcher path —
//! payload decoding stays in the plugin's
//! [`sonic_connector_transport_iox::ChannelReader::try_recv`].

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use sonic_connector_core::ConnectorError;
use sonic_connector_transport_iox::{RawChannelReader, RawChannelWriter};

use crate::driver::BusDriver;
use crate::pdi;
use crate::registry::{
    ChannelBinding, ChannelRegistry, InboundPublish, OutboundDrain, RegisteredChannel,
};
use crate::routing::PdoDirection;
use crate::runner::{CycleReport, CycleRunner};

/// Stack-allocated scratch size used by the inbound path.
///
/// The dispatcher reuses a single fixed-size buffer per cycle to copy
/// PDI bit-slice bytes out of the driver and into the gateway-side
/// iceoryx2 publisher. Channels whose `bit_length / 8` exceeds this
/// surface as [`ConnectorError::PayloadOverflow`].
pub const MAX_INBOUND_PAYLOAD: usize = 4096;

/// Trait-object wrapper around a gateway-side [`RawChannelReader`].
///
/// Implements [`OutboundDrain`]. The caller-supplied `dest` slice
/// doubles as the iceoryx2 receive buffer, so no internal scratch
/// is needed (the dispatcher carries one stack buffer per cycle).
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

/// Trait-object wrapper around a gateway-side [`RawChannelWriter`]
/// — implements [`InboundPublish`].
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

/// Outcome of one [`dispatch_one_cycle`] call.
///
/// The `cycle` field is `None` when the scheduler decided to skip the
/// tick (`REQ_0317`) — outbound bytes were still moved to PDI, but no
/// `tx_rx` happened and no inbound bytes were published.
#[derive(Debug)]
pub struct DispatchReport {
    /// Number of outbound envelopes drained from iceoryx2 and written
    /// to PDI this cycle.
    pub outbound_envelopes: usize,
    /// Number of inbound envelopes published to iceoryx2 this cycle.
    pub inbound_envelopes: usize,
    /// The cycle report from the scheduler, if a cycle fired.
    pub cycle: Option<CycleReport>,
}

/// Run one dispatcher iteration:
///
/// 1. For every outbound channel, drain pending iceoryx2 envelopes
///    and write the bytes into the SubDevice's outputs PDI slice
///    via [`pdi::write_routing`] (`REQ_0326`).
/// 2. Call [`CycleRunner::tick`] to perform the cycle's `tx_rx` and
///    update the WKC verdict (`REQ_0317`, `REQ_0319`).
/// 3. For every inbound channel, read the SubDevice's inputs PDI
///    slice via [`pdi::read_routing`] into a scratch buffer and
///    publish those bytes on the channel's inbound iceoryx2 service
///    (`REQ_0327`).
///
/// The registry mutex is released before the async `tick` and
/// re-acquired for the inbound pass so plugin-side
/// `create_writer` / `create_reader` calls can still register new
/// channels while a cycle is in flight (registrations are append-only,
/// so a concurrent registration during dispatch is consistent —
/// the new channel becomes visible to the next cycle).
///
/// # Errors
///
/// Propagates any error from a binding's drain / publish call, from
/// `pdi::write_routing` / `pdi::read_routing`, or from the
/// [`CycleRunner::tick`]. The dispatcher's failure mode is
/// best-effort: a single channel's error fails the whole cycle so the
/// caller sees the diagnostic.
pub async fn dispatch_one_cycle<D>(
    registry: &Mutex<ChannelRegistry>,
    runner: &mut CycleRunner<D>,
    now: Instant,
) -> Result<DispatchReport, ConnectorError>
where
    D: BusDriver,
{
    let outbound_envelopes = dispatch_outbound(registry, runner)?;
    let cycle = runner.tick(now).await?;
    let inbound_envelopes = if cycle.is_some() {
        dispatch_inbound(registry, runner)?
    } else {
        0
    };
    Ok(DispatchReport {
        outbound_envelopes,
        inbound_envelopes,
        cycle,
    })
}

#[allow(clippy::significant_drop_tightening)] // the guard is legitimately
// held across the loop; releasing it mid-iteration would let a
// concurrent registration interleave a partially-applied channel.
fn dispatch_outbound<D>(
    registry: &Mutex<ChannelRegistry>,
    runner: &CycleRunner<D>,
) -> Result<usize, ConnectorError>
where
    D: BusDriver,
{
    let guard = registry.lock().expect("registry mutex not poisoned");
    let mut envelopes = 0_usize;
    for channel in guard.iter() {
        let RegisteredChannel {
            routing,
            direction,
            binding,
            ..
        } = channel;
        if *direction != PdoDirection::Rx {
            continue;
        }
        let ChannelBinding::Outbound(drain) = binding else {
            continue;
        };
        // Drain every pending envelope on this channel into the
        // SubDevice's outputs slice. The driver's
        // `with_subdevice_outputs_mut` callback shape keeps the
        // ethercrab guard scoped to a single envelope; we re-enter
        // the callback per envelope so the guard's lifetime stays
        // bounded.
        loop {
            // First peek: drain one envelope into a stack buffer
            // sized to a generous workspace max. The per-binding
            // scratch is consulted inside `drain_into`.
            let mut payload = [0u8; MAX_INBOUND_PAYLOAD];
            let Some(written) = drain.drain_into(&mut payload)? else {
                break;
            };
            let outcome = runner
                .driver()
                .with_subdevice_outputs_mut(routing.subdevice_address, |pdi_buf| {
                    pdi::write_routing(pdi_buf, routing, &payload[..written])
                });
            match outcome {
                Some(Ok(())) => {
                    envelopes += 1;
                }
                Some(Err(e)) => return Err(e),
                None => {
                    return Err(ConnectorError::stack(MissingSubdevice {
                        address: routing.subdevice_address,
                        direction: "outputs",
                    }));
                }
            }
        }
    }
    Ok(envelopes)
}

#[allow(clippy::significant_drop_tightening)] // same as dispatch_outbound:
// guard is held across the iteration.
fn dispatch_inbound<D>(
    registry: &Mutex<ChannelRegistry>,
    runner: &CycleRunner<D>,
) -> Result<usize, ConnectorError>
where
    D: BusDriver,
{
    let guard = registry.lock().expect("registry mutex not poisoned");
    let mut envelopes = 0_usize;
    for channel in guard.iter() {
        let RegisteredChannel {
            routing,
            direction,
            binding,
            ..
        } = channel;
        if *direction != PdoDirection::Tx {
            continue;
        }
        let ChannelBinding::Inbound(publish) = binding else {
            continue;
        };
        let bit_length = routing.bit_length as usize;
        if bit_length == 0 {
            continue;
        }
        let payload_len = bit_length.div_ceil(8);
        if payload_len > MAX_INBOUND_PAYLOAD {
            return Err(ConnectorError::PayloadOverflow {
                actual: payload_len,
                max: MAX_INBOUND_PAYLOAD,
            });
        }
        let mut scratch = [0u8; MAX_INBOUND_PAYLOAD];
        let outcome = runner
            .driver()
            .with_subdevice_inputs(routing.subdevice_address, |pdi_buf| {
                pdi::read_routing(pdi_buf, routing, &mut scratch[..payload_len])
            });
        match outcome {
            Some(Ok(())) => {}
            Some(Err(e)) => return Err(e),
            None => {
                return Err(ConnectorError::stack(MissingSubdevice {
                    address: routing.subdevice_address,
                    direction: "inputs",
                }));
            }
        }
        publish.publish_bytes(&scratch[..payload_len])?;
        envelopes += 1;
    }
    Ok(envelopes)
}

/// Long-running dispatcher loop. Spawned by
/// `EthercatConnector::register_with` on the gateway's tokio
/// runtime. The loop checks `stop` every iteration and exits cleanly
/// when it flips to `true`.
///
/// `cycle_period` paces the loop — between cycles the task sleeps to
/// avoid spinning the CPU when the scheduler is going to `Skip` the
/// next tick anyway.
///
/// # Errors
///
/// Returns the first error from any [`dispatch_one_cycle`] call.
/// Production callers typically log the error and let the task exit;
/// the [`crate::EthercatHealthMonitor`] surfaces the cycle outcome
/// separately via `Degraded` transitions.
pub async fn dispatcher_loop<D>(
    registry: Arc<Mutex<ChannelRegistry>>,
    mut runner: CycleRunner<D>,
    stop: Arc<AtomicBool>,
    cycle_period: std::time::Duration,
) -> Result<(), ConnectorError>
where
    D: BusDriver,
{
    while !stop.load(Ordering::Acquire) {
        dispatch_one_cycle(&registry, &mut runner, Instant::now()).await?;
        tokio::time::sleep(cycle_period).await;
    }
    Ok(())
}

#[derive(Debug)]
struct MissingSubdevice {
    address: u16,
    direction: &'static str,
}

impl core::fmt::Display for MissingSubdevice {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "ethercat dispatcher: no SubDevice at address 0x{:04x} for {} PDI access",
            self.address, self.direction
        )
    }
}

impl std::error::Error for MissingSubdevice {}
