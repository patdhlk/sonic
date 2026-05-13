//! Pub/sub dispatcher loop and iceoryx2 adapters.
//!
//! The dispatcher runs on the gateway's tokio runtime once
//! [`crate::gateway::ZenohGateway`] is started. It iterates the
//! channel registry on each tick — for outbound bindings, it drains
//! the iceoryx2 raw subscriber and forwards bytes to
//! `session.publish`. Inbound bindings are driven by the session's
//! subscribe callbacks set up at `create_reader` time (see
//! [`IoxInboundPublish`]); the loop does not iterate them.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sonic_connector_core::ConnectorError;
use sonic_connector_transport_iox::{RawChannelReader, RawChannelWriter};

use crate::registry::{
    ChannelBinding, ChannelRegistry, CorrelatedPublish, InboundPublish, OutboundDrain, QuerierDrain,
    QueryId, ReplyDrain,
};
use crate::session::ZenohSessionLike;

/// Maximum scratch-buffer size the dispatcher allocates per drain
/// (heap-allocated once at loop entry). Channels with `N >
/// MAX_DRAIN_SCRATCH` will fail the drain step; tune up if needed.
const MAX_DRAIN_SCRATCH: usize = 4096;

/// iceoryx2 outbound drain — wraps a [`RawChannelReader<N>`].
///
/// Implements [`OutboundDrain`] so the dispatcher can drain bytes from
/// the iceoryx2 raw subscriber as a trait object, erasing the const
/// generic `N` from the registry.
pub struct IoxOutboundDrain<const N: usize> {
    reader: RawChannelReader<N>,
}

impl<const N: usize> IoxOutboundDrain<N> {
    /// Wrap a `RawChannelReader` so the dispatcher can drain it as a
    /// trait object.
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

/// iceoryx2 inbound publisher — wraps a [`RawChannelWriter<N>`].
///
/// Wrapped in a `Mutex` so concurrent session callbacks can use the
/// same publisher via `&self`. [`RawChannelWriter`] is `Send` but not
/// `Sync`; the `Mutex` provides the interior-mutability needed to
/// satisfy `InboundPublish`'s `Send + Sync` bound.
pub struct IoxInboundPublish<const N: usize> {
    writer: Mutex<RawChannelWriter<N>>,
}

impl<const N: usize> IoxInboundPublish<N> {
    /// Wrap a `RawChannelWriter` so session callbacks can republish
    /// bytes through it.
    #[must_use]
    pub const fn new(writer: RawChannelWriter<N>) -> Self {
        Self {
            writer: Mutex::new(writer),
        }
    }
}

impl<const N: usize> InboundPublish for IoxInboundPublish<N> {
    fn publish_bytes(&self, bytes: &[u8]) -> Result<(), ConnectorError> {
        let writer = self.writer.lock().expect("inbound publisher mutex poisoned");
        writer.send_raw_bytes(bytes, [0u8; 32]).map(|_| ())
    }
}

/// Dispatcher loop.
///
/// Drains all outbound channels and forwards their payloads to the
/// session. Inbound channels are not iterated here — their delivery is
/// driven by session callbacks registered at `create_reader` time.
///
/// Runs until `stop.load(Ordering::Acquire)` is `true`. Sleeps
/// `tick_interval` between drains.
///
/// # Errors
///
/// Returns the first non-recoverable error encountered. Per-iteration
/// `ConnectorError::BackPressure` / iceoryx2 receive failures do not
/// abort the loop.
pub async fn dispatcher_loop<S>(
    registry: Arc<Mutex<ChannelRegistry>>,
    session: Arc<S>,
    stop: Arc<AtomicBool>,
    tick_interval: Duration,
) -> Result<(), ConnectorError>
where
    S: ZenohSessionLike + ?Sized,
{
    let mut scratch = vec![0u8; MAX_DRAIN_SCRATCH];
    while !stop.load(Ordering::Acquire) {
        drain_outbound_once(&registry, session.as_ref(), &mut scratch);
        tokio::time::sleep(tick_interval).await;
    }
    Ok(())
}

fn drain_outbound_once<S>(
    registry: &Mutex<ChannelRegistry>,
    session: &S,
    scratch: &mut [u8],
) where
    S: ZenohSessionLike + ?Sized,
{
    let guard = registry.lock().expect("registry mutex poisoned");
    for entry in guard.iter() {
        if let ChannelBinding::Outbound(drain) = &entry.binding {
            while let Ok(Some(n)) = drain.drain_into(scratch) {
                let _ = session.publish(&entry.routing, &scratch[..n]);
            }
        }
    }
}

/// iox-backed [`QuerierDrain`]. Drains envelopes from `.query.out`,
/// returning `(QueryId, payload_len)`.
pub struct IoxQuerierDrain<const N: usize> {
    reader: RawChannelReader<N>,
}

impl<const N: usize> IoxQuerierDrain<N> {
    /// Wrap a [`RawChannelReader`] so the dispatcher can drain it as a
    /// trait object.
    #[must_use]
    pub const fn new(reader: RawChannelReader<N>) -> Self {
        Self { reader }
    }
}

impl<const N: usize> QuerierDrain for IoxQuerierDrain<N> {
    fn drain_query(
        &self,
        dest: &mut [u8],
    ) -> Result<Option<(QueryId, usize)>, ConnectorError> {
        let Some(sample) = self.reader.try_recv_into(dest)? else {
            return Ok(None);
        };
        Ok(Some((QueryId(sample.correlation_id), sample.payload_len)))
    }
}

/// iox-backed [`ReplyDrain`]. Drains envelopes from `.reply.out`,
/// returning `(QueryId, payload_len)`.
pub struct IoxReplyDrain<const N: usize> {
    reader: RawChannelReader<N>,
}

impl<const N: usize> IoxReplyDrain<N> {
    /// Wrap a [`RawChannelReader`] so the dispatcher can drain it as a
    /// trait object.
    #[must_use]
    pub const fn new(reader: RawChannelReader<N>) -> Self {
        Self { reader }
    }
}

impl<const N: usize> ReplyDrain for IoxReplyDrain<N> {
    fn drain_reply(
        &self,
        dest: &mut [u8],
    ) -> Result<Option<(QueryId, usize)>, ConnectorError> {
        let Some(sample) = self.reader.try_recv_into(dest)? else {
            return Ok(None);
        };
        Ok(Some((QueryId(sample.correlation_id), sample.payload_len)))
    }
}

/// iox-backed [`CorrelatedPublish`]. Publishes bytes verbatim with the
/// caller-supplied `correlation_id`. Mutex-wrapped because session
/// callbacks invoke this from multiple threads.
pub struct IoxCorrelatedPublish<const N: usize> {
    writer: Mutex<RawChannelWriter<N>>,
}

impl<const N: usize> IoxCorrelatedPublish<N> {
    /// Wrap a [`RawChannelWriter`] so session callbacks can publish
    /// bytes through it with explicit correlation ids.
    #[must_use]
    pub const fn new(writer: RawChannelWriter<N>) -> Self {
        Self {
            writer: Mutex::new(writer),
        }
    }
}

impl<const N: usize> CorrelatedPublish for IoxCorrelatedPublish<N> {
    fn publish_with_correlation(
        &self,
        id: QueryId,
        bytes: &[u8],
    ) -> Result<(), ConnectorError> {
        let writer = self.writer.lock().expect("correlated publisher mutex poisoned");
        writer.send_raw_bytes(bytes, id.0).map(|_| ())
    }
}
