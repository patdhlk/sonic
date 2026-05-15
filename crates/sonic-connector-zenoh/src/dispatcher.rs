//! Pub/sub dispatcher loop and iceoryx2 adapters.
//!
//! The dispatcher runs on the gateway's tokio runtime once
//! [`crate::gateway::ZenohGateway`] is started. It iterates the
//! channel registry on each tick — for outbound bindings, it drains
//! the iceoryx2 raw subscriber and forwards bytes to
//! `session.publish`. Inbound bindings are driven by the session's
//! subscribe callbacks set up at `create_reader` time (see
//! [`IoxInboundPublish`]); the loop does not iterate them.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sonic_connector_core::ConnectorError;
use sonic_connector_transport_iox::{RawChannelReader, RawChannelWriter};
use tracing::{debug, warn};

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

/// Sidecar set — correlation IDs whose reply path is sealed because
/// the gateway emitted a synthetic `[0x03]` terminator on timeout.
/// Reply / done closures in `spawn_query_with_timeout` consult this
/// set before publishing any frame and silently drop the frame if the
/// id is present. Entries evict after one more `effective_timeout`
/// so the set remains bounded (`Z5c`).
pub(crate) type SealedQueries = Arc<Mutex<HashSet<QueryId>>>;

/// Maximum scratch-buffer size the dispatcher allocates per drain
/// (heap-allocated once at loop entry). Channels with `N >
/// MAX_DRAIN_SCRATCH` will fail the drain step; tune up if needed.
const MAX_DRAIN_SCRATCH: usize = 4096;

/// iceoryx2 outbound drain — wraps a [`RawChannelReader<N>`].
///
/// Implements [`OutboundDrain`] so the dispatcher can drain bytes from
/// the iceoryx2 raw subscriber as a trait object, erasing the const
/// generic `N` from the registry.
///
/// The reader is `Mutex`-wrapped to give the drain interior mutability
/// behind a `Send + Sync` surface — [`RawChannelReader`] is `Send` but
/// not `Sync`, and Z4a's snapshot pattern stores drains as
/// `Arc<dyn OutboundDrain>` (which requires `Send + Sync`).
pub struct IoxOutboundDrain<const N: usize> {
    reader: Mutex<RawChannelReader<N>>,
}

impl<const N: usize> IoxOutboundDrain<N> {
    /// Wrap a `RawChannelReader` so the dispatcher can drain it as a
    /// trait object.
    #[must_use]
    pub const fn new(reader: RawChannelReader<N>) -> Self {
        Self {
            reader: Mutex::new(reader),
        }
    }
}

impl<const N: usize> OutboundDrain for IoxOutboundDrain<N> {
    fn drain_into(&self, dest: &mut [u8]) -> Result<Option<usize>, ConnectorError> {
        let sample_opt = {
            let reader = self.reader.lock().expect("outbound drain mutex poisoned");
            reader.try_recv_into(dest)?
        };
        let Some(sample) = sample_opt else {
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
    sealed_queries: SealedQueries,
    query_timeout: Duration,
) -> Result<(), ConnectorError>
where
    S: ZenohSessionLike + 'static,
{
    let mut scratch = vec![0u8; MAX_DRAIN_SCRATCH];
    while !stop.load(Ordering::Acquire) {
        drain_outbound_once(
            &registry,
            &session,
            &mut scratch,
            &correlation_map,
            &query_reply_publishers,
            &sealed_queries,
            query_timeout,
        )
        .await;
        tokio::time::sleep(tick_interval).await;
    }
    Ok(())
}

/// Per-iteration snapshot of one registry entry. Built under the
/// registry lock and then iterated lock-free so the async session
/// calls below never hold the registry mutex across an `.await`.
struct RegistrySnapshot {
    descriptor_name: std::borrow::Cow<'static, str>,
    routing: crate::routing::ZenohRouting,
    binding: BindingSnapshot,
}

/// Lock-snapshot mirror of [`ChannelBinding`]. Publish-side bindings
/// (`Inbound`, `QuerierReplyIn`, `QueryableQueryIn`) collapse into
/// [`BindingSnapshot::PublishSide`] because the dispatcher does not
/// iterate them — those bindings are driven by session callbacks at
/// registration time.
enum BindingSnapshot {
    Outbound(Arc<dyn OutboundDrain>),
    QuerierOut(Arc<dyn QuerierDrain>),
    QueryableReplyOut(Arc<dyn ReplyDrain>),
    PublishSide,
}

impl RegistrySnapshot {
    fn clone_arcs(entry: &crate::registry::RegisteredChannel) -> Self {
        let binding = match &entry.binding {
            ChannelBinding::Outbound(d) => BindingSnapshot::Outbound(Arc::clone(d)),
            ChannelBinding::QuerierOut(d) => BindingSnapshot::QuerierOut(Arc::clone(d)),
            ChannelBinding::QueryableReplyOut(d) => {
                BindingSnapshot::QueryableReplyOut(Arc::clone(d))
            }
            ChannelBinding::Inbound(_)
            | ChannelBinding::QuerierReplyIn(_)
            | ChannelBinding::QueryableQueryIn(_) => BindingSnapshot::PublishSide,
        };
        Self {
            descriptor_name: entry.descriptor_name.clone(),
            routing: entry.routing.clone(),
            binding,
        }
    }
}

#[allow(clippy::too_many_arguments)] // each arg is inherent to the dispatcher's responsibilities.
async fn drain_outbound_once<S>(
    registry: &Mutex<ChannelRegistry>,
    session: &Arc<S>,
    scratch: &mut [u8],
    correlation_map: &CorrelationMap,
    query_reply_publishers: &QueryReplyPublishers,
    sealed_queries: &SealedQueries,
    query_timeout: Duration,
) where
    S: ZenohSessionLike + 'static,
{
    // Snapshot the registry under the lock, then iterate lock-free.
    // This is mandatory after Z4a: holding `MutexGuard<ChannelRegistry>`
    // across `.await` would trip clippy::await_holding_lock AND
    // deadlock against any caller that needs the registry while a
    // session call is in flight.
    let entries: Vec<RegistrySnapshot> = {
        let guard = registry.lock().expect("registry mutex poisoned");
        guard.iter().map(RegistrySnapshot::clone_arcs).collect()
    };

    for entry in entries {
        match entry.binding {
            BindingSnapshot::Outbound(drain) => {
                while let Ok(Some(n)) = drain.drain_into(scratch) {
                    if let Err(e) = session.publish(&entry.routing, &scratch[..n]).await {
                        warn!(
                            descriptor = %entry.descriptor_name,
                            error = %e,
                            "session.publish failed; dropping outbound chunk"
                        );
                    }
                }
            }
            BindingSnapshot::QuerierOut(drain) => {
                while let Ok(Some((id, n, reserved))) = drain.drain_query(scratch) {
                    // Resolve the effective timeout: `reserved == 0`
                    // means "use the connector default" (REQ_0425).
                    let effective_timeout = if reserved == 0 {
                        query_timeout
                    } else {
                        Duration::from_millis(u64::from(reserved))
                    };
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
                    spawn_query_with_timeout(
                        Arc::clone(session),
                        entry.routing.clone(),
                        scratch[..n].to_vec(),
                        id,
                        effective_timeout,
                        publisher,
                        Arc::clone(sealed_queries),
                    );
                }
            }
            BindingSnapshot::QueryableReplyOut(drain) => {
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
                        Some(crate::session::FrameKind::Timeout) => {
                            // 0x03 should never come from the plugin
                            // (gateway-synthetic only).
                            warn!(
                                ?id,
                                "unexpected 0x03 frame on .reply.out (gateway-synthetic only)"
                            );
                        }
                        None => {
                            warn!(
                                discriminator,
                                ?id,
                                "unknown frame discriminator on .reply.out"
                            );
                        }
                    }
                }
            }
            BindingSnapshot::PublishSide => {
                // Publish-side bindings — driven by session callbacks
                // at registration time, not by the dispatcher loop.
            }
        }
    }
}

/// Spawn one upstream `session.query` wrapped in `tokio::time::timeout`.
///
/// The dispatcher uses this helper for every plugin-issued query so it
/// can move on to drain the next entry without blocking — the timeout
/// must still fire even if `session.query` never resolves (e.g. real
/// zenoh sends out the query but no peer replies; `REQ_0425` /
/// `TEST_0307`).
///
/// On timeout expiry, a synthetic `[0x03]` (`FrameKind::Timeout`)
/// frame is published on the matching `.reply.in` channel so the
/// querier observes `QuerierEvent::Timeout`.
///
/// Late zenoh replies (real-session only) are filtered by the
/// `sealed_queries` sidecar — see `on_reply` / `on_done` closures
/// above. The timeout path seals the id BEFORE publishing the `0x03`
/// so a racy upstream callback observes the sealed state and drops
/// its frame. A delayed eviction task removes the id after one more
/// `effective_timeout` so the set stays bounded (`Z5c`).
fn spawn_query_with_timeout<S>(
    session: Arc<S>,
    routing: crate::routing::ZenohRouting,
    payload: Vec<u8>,
    id: QueryId,
    effective_timeout: Duration,
    publisher: Arc<dyn CorrelatedPublish>,
    sealed_queries: SealedQueries,
) where
    S: ZenohSessionLike + 'static,
{
    let pub_reply = Arc::clone(&publisher);
    let pub_done = Arc::clone(&publisher);
    // The third use moves the Arc into the spawned future — keeps
    // clippy::needless_pass_by_value happy and avoids one needless
    // clone on the hot path.
    let publisher_for_timeout = publisher;
    let sealed_for_reply = Arc::clone(&sealed_queries);
    let sealed_for_done = Arc::clone(&sealed_queries);
    let sealed_for_evict = Arc::clone(&sealed_queries);
    tokio::spawn(async move {
        let on_reply: PayloadSink = Box::new(move |bytes: &[u8]| {
            // Drop late replies that arrived after the gateway sealed
            // this id on timeout. Hoist the lookup so we never hold
            // the mutex across the publish.
            let sealed = sealed_for_reply
                .lock()
                .expect("sealed_queries poisoned")
                .contains(&id);
            if sealed {
                return;
            }
            let mut framed = Vec::with_capacity(1 + bytes.len());
            framed.push(crate::session::FrameKind::Data.discriminator());
            framed.extend_from_slice(bytes);
            let _ = pub_reply.publish_with_correlation(id, &framed);
        });
        let on_done: DoneCallback = Box::new(move || {
            let sealed = sealed_for_done
                .lock()
                .expect("sealed_queries poisoned")
                .contains(&id);
            if sealed {
                return;
            }
            let _ = pub_done.publish_with_correlation(
                id,
                &[crate::session::FrameKind::EndOfStream.discriminator()],
            );
        });
        let query_fut =
            session.query(&routing, &payload, effective_timeout, on_reply, on_done);
        match tokio::time::timeout(effective_timeout, query_fut).await {
            Ok(Ok(())) => {
                debug!(query_id = ?id, "query completed");
            }
            Ok(Err(e)) => {
                warn!(query_id = ?id, error = %e, "session.query returned error");
            }
            Err(_elapsed) => {
                // Timeout fired before session.query completed — emit
                // the synthetic 0x03 terminator on the reply path so
                // the querier sees `QuerierEvent::Timeout` (TEST_0307).
                //
                // CRITICAL: seal BEFORE publishing so any upstream
                // callback racing with this branch observes the seal
                // and drops its frame.
                sealed_queries
                    .lock()
                    .expect("sealed_queries poisoned")
                    .insert(id);
                warn!(
                    query_id = ?id,
                    ?effective_timeout,
                    "query timed out, emitting 0x03"
                );
                let _ = publisher_for_timeout.publish_with_correlation(
                    id,
                    &[crate::session::FrameKind::Timeout.discriminator()],
                );
                // Bounded eviction: drop the seal after another
                // `effective_timeout` so the set doesn't grow without
                // bound. Fire-and-forget — the JoinHandle is dropped.
                let evict_after = effective_timeout;
                let _evict = tokio::spawn(async move {
                    tokio::time::sleep(evict_after).await;
                    sealed_for_evict
                        .lock()
                        .expect("sealed_queries poisoned")
                        .remove(&id);
                });
            }
        }
    });
}

/// iox-backed [`QuerierDrain`]. Drains envelopes from `.query.out`,
/// returning `(QueryId, payload_len)`.
///
/// `Mutex`-wrapped for `Send + Sync` — see [`IoxOutboundDrain`].
pub struct IoxQuerierDrain<const N: usize> {
    reader: Mutex<RawChannelReader<N>>,
}

impl<const N: usize> IoxQuerierDrain<N> {
    /// Wrap a [`RawChannelReader`] so the dispatcher can drain it as a
    /// trait object.
    #[must_use]
    pub const fn new(reader: RawChannelReader<N>) -> Self {
        Self {
            reader: Mutex::new(reader),
        }
    }
}

impl<const N: usize> QuerierDrain for IoxQuerierDrain<N> {
    fn drain_query(
        &self,
        dest: &mut [u8],
    ) -> Result<Option<(QueryId, usize, u32)>, ConnectorError> {
        let sample_opt = {
            let reader = self.reader.lock().expect("querier drain mutex poisoned");
            reader.try_recv_into(dest)?
        };
        let Some(sample) = sample_opt else {
            return Ok(None);
        };
        Ok(Some((
            QueryId(sample.correlation_id),
            sample.payload_len,
            sample.reserved,
        )))
    }
}

/// iox-backed [`ReplyDrain`]. Drains envelopes from `.reply.out`,
/// returning `(QueryId, payload_len)`.
///
/// `Mutex`-wrapped for `Send + Sync` — see [`IoxOutboundDrain`].
pub struct IoxReplyDrain<const N: usize> {
    reader: Mutex<RawChannelReader<N>>,
}

impl<const N: usize> IoxReplyDrain<N> {
    /// Wrap a [`RawChannelReader`] so the dispatcher can drain it as a
    /// trait object.
    #[must_use]
    pub const fn new(reader: RawChannelReader<N>) -> Self {
        Self {
            reader: Mutex::new(reader),
        }
    }
}

impl<const N: usize> ReplyDrain for IoxReplyDrain<N> {
    fn drain_reply(
        &self,
        dest: &mut [u8],
    ) -> Result<Option<(QueryId, usize)>, ConnectorError> {
        let sample_opt = {
            let reader = self.reader.lock().expect("reply drain mutex poisoned");
            reader.try_recv_into(dest)?
        };
        let Some(sample) = sample_opt else {
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
