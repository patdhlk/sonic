//! Pub/sub dispatcher loop and iceoryx2 adapters.
//!
//! The dispatcher runs on the gateway's tokio runtime once
//! [`crate::gateway::ZenohGateway`] is started. It iterates the
//! channel registry on each tick — for outbound bindings, it drains
//! the iceoryx2 raw subscriber and forwards bytes to
//! `session.publish`. Inbound bindings are driven by the session's
//! subscribe callbacks set up at `create_reader` time (see
//! [`IoxInboundPublish`]); the loop does not iterate them.

use std::collections::HashMap;
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
use crate::session::{DoneCallback, PayloadSink, QueryReplier, ZenohSessionLike};

/// Shared map of in-flight upstream queries — gateway-minted
/// [`QueryId`] → [`QueryReplier`] from the upstream session.
///
/// Populated by `create_queryable`'s session callback; consumed by the
/// dispatcher when draining `.reply.out` so it can forward chunks back
/// to the originating upstream querier.
pub(crate) type CorrelationMap = Arc<Mutex<HashMap<QueryId, QueryReplier>>>;

/// Sidecar map (Option B from the Z3 plan) — descriptor name →
/// `.reply.in` publisher. Lets the dispatcher's `QuerierOut` branch
/// look up the matching reply publisher without re-entering the
/// registry mutex and without juggling generics through the registry.
pub(crate) type QueryReplyPublishers =
    Arc<Mutex<HashMap<String, Arc<dyn CorrelatedPublish>>>>;

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
/// Drains all outbound and query-side channels and forwards their
/// payloads to / from the session:
///
/// * [`ChannelBinding::Outbound`] — bytes → `session.publish`.
/// * [`ChannelBinding::QuerierOut`] — bytes → `session.query` with
///   reply-stamping callbacks bound to the matching `.reply.in`
///   publisher (looked up by descriptor name in `query_reply_publishers`).
/// * [`ChannelBinding::QueryableReplyOut`] — framed bytes →
///   `QueryReplier::reply` / `QueryReplier::terminate` via
///   `correlation_map`.
///
/// Inbound, `QuerierReplyIn`, and `QueryableQueryIn` bindings are NOT
/// iterated here — their delivery is driven by session callbacks
/// registered at `create_reader` / `create_queryable` time.
///
/// Runs until `stop.load(Ordering::Acquire)` is `true`. Sleeps
/// `tick_interval` between drains.
///
/// # Errors
///
/// Returns the first non-recoverable error encountered. Per-iteration
/// `ConnectorError::BackPressure` / iceoryx2 receive failures do not
/// abort the loop.
#[allow(clippy::too_many_arguments)] // each arg is inherent to the dispatcher's responsibilities.
pub async fn dispatcher_loop<S>(
    registry: Arc<Mutex<ChannelRegistry>>,
    session: Arc<S>,
    stop: Arc<AtomicBool>,
    tick_interval: Duration,
    correlation_map: CorrelationMap,
    query_reply_publishers: QueryReplyPublishers,
    query_timeout: Duration,
) -> Result<(), ConnectorError>
where
    S: ZenohSessionLike + ?Sized,
{
    let mut scratch = vec![0u8; MAX_DRAIN_SCRATCH];
    while !stop.load(Ordering::Acquire) {
        drain_outbound_once(
            &registry,
            session.as_ref(),
            &mut scratch,
            &correlation_map,
            &query_reply_publishers,
            query_timeout,
        );
        tokio::time::sleep(tick_interval).await;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)] // each arg is inherent to the dispatcher's responsibilities.
fn drain_outbound_once<S>(
    registry: &Mutex<ChannelRegistry>,
    session: &S,
    scratch: &mut [u8],
    correlation_map: &CorrelationMap,
    query_reply_publishers: &QueryReplyPublishers,
    query_timeout: Duration,
) where
    S: ZenohSessionLike + ?Sized,
{
    let guard = registry.lock().expect("registry mutex poisoned");
    for entry in guard.iter() {
        match &entry.binding {
            ChannelBinding::Outbound(drain) => {
                while let Ok(Some(n)) = drain.drain_into(scratch) {
                    let _ = session.publish(&entry.routing, &scratch[..n]);
                }
            }
            ChannelBinding::QuerierOut(drain) => {
                while let Ok(Some((id, n))) = drain.drain_query(scratch) {
                    // Look up the matching `.reply.in` publisher in
                    // the sidecar map; release the lock immediately so
                    // the inner publish path doesn't hold it.
                    let publisher_opt = {
                        let map = query_reply_publishers
                            .lock()
                            .expect("query reply publishers poisoned");
                        map.get(entry.descriptor_name.as_ref()).map(Arc::clone)
                    };
                    let Some(publisher) = publisher_opt else {
                        // No reply path registered — drop the query
                        // (the plugin will time out anyway).
                        continue;
                    };
                    let payload = scratch[..n].to_vec();
                    let pub_reply = Arc::clone(&publisher);
                    let pub_done = Arc::clone(&publisher);
                    let on_reply: PayloadSink = Box::new(move |bytes: &[u8]| {
                        let mut framed = Vec::with_capacity(1 + bytes.len());
                        framed.push(crate::session::FrameKind::Data.discriminator());
                        framed.extend_from_slice(bytes);
                        let _ = pub_reply.publish_with_correlation(id, &framed);
                    });
                    let on_done: DoneCallback = Box::new(move || {
                        let _ = pub_done.publish_with_correlation(
                            id,
                            &[crate::session::FrameKind::EndOfStream.discriminator()],
                        );
                    });
                    // For Z3, the mock's `session.query` is synchronous
                    // and returns immediately. Timeout enforcement
                    // (0x03 synthetic terminator) is deferred to Z4 —
                    // when the real `zenoh::Session` lands, we'll wrap
                    // this in `tokio::time::timeout`. For now, we pass
                    // the timeout through but the mock ignores it.
                    let _ = session.query(
                        &entry.routing,
                        &payload,
                        query_timeout,
                        on_reply,
                        on_done,
                    );
                }
            }
            ChannelBinding::QueryableReplyOut(drain) => {
                while let Ok(Some((id, n))) = drain.drain_reply(scratch) {
                    if n == 0 {
                        continue;
                    }
                    let discriminator = scratch[0];
                    match crate::session::FrameKind::from_byte(discriminator) {
                        Some(crate::session::FrameKind::Data) => {
                            // Data chunk: forward body to the upstream
                            // replier under the correlation map lock.
                            let map = correlation_map
                                .lock()
                                .expect("correlation map poisoned");
                            if let Some(replier) = map.get(&id) {
                                replier.reply(&scratch[1..n]);
                            }
                        }
                        Some(crate::session::FrameKind::EndOfStream) => {
                            // EoS: remove the replier and finalise.
                            let replier = correlation_map
                                .lock()
                                .expect("correlation map poisoned")
                                .remove(&id);
                            if let Some(replier) = replier {
                                replier.terminate();
                            }
                        }
                        Some(crate::session::FrameKind::Timeout) | None => {
                            // 0x03 should never come from the plugin
                            // (gateway-synthetic only). Unknown
                            // discriminators are silently dropped —
                            // Z4 adds logging.
                        }
                    }
                }
            }
            ChannelBinding::Inbound(_)
            | ChannelBinding::QuerierReplyIn(_)
            | ChannelBinding::QueryableQueryIn(_) => {
                // Publish-side bindings — driven by session callbacks
                // at registration time, not by the dispatcher loop.
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
