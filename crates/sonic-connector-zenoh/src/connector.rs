//! [`ZenohConnector`] — plugin-side implementation of the framework's
//! `Connector` trait (`REQ_0400`).
//!
//! Generic over a [`ZenohSessionLike`] back-end (the session — mock
//! in tests, real in Z4) and a `PayloadCodec` (`REQ_0211`).
//!
//! Z2 Task 5 lands the struct + constructor; Z2 Task 6 adds the full
//! `Connector` trait impl (`name`, `health`, `subscribe_health`,
//! `register_with`, `create_writer`, `create_reader`).

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Type-erased owning handle for session-side declarations
/// (subscriber, queryable). Stored in [`ZenohState::handles`]; on
/// connector drop the bag drops, each `Box<dyn Any + Send + Sync>`
/// drops, and each handle's `Drop` impl releases its session-side
/// resource.
type AnyHandle = Box<dyn Any + Send + Sync>;

use iceoryx2::node::Node;
use iceoryx2::prelude::{NodeBuilder, ipc};
use sonic_connector_core::ConnectorError;
use sonic_connector_transport_iox::ServiceFactory;

use crate::dispatcher::{CorrelationMap, QueryReplyPublishers, SealedQueries};
use crate::gateway::ZenohGateway;
use crate::health::ZenohHealthMonitor;
use crate::options::ZenohConnectorOptions;
use crate::registry::{ChannelRegistry, QueryId};
use crate::session::{SessionState, ZenohSessionLike};

/// Connector-internal state shared between [`ZenohConnector`] and the
/// gateway-side dispatcher.
pub struct ZenohState {
    health: Arc<ZenohHealthMonitor>,
    options: ZenohConnectorOptions,
    registry: Arc<Mutex<ChannelRegistry>>,
    stop: Arc<AtomicBool>,
    correlation_map: CorrelationMap,
    query_reply_publishers: QueryReplyPublishers,
    /// Type-erased bag of session-declaration handles
    /// (subscriber, queryable). Held for the lifetime of the
    /// connector — Z4d's lifecycle story: when the connector drops,
    /// `Arc<ZenohState>` reaches strong-count 0, the bag drops, each
    /// handle's `Drop` impl tears down its session-side resource.
    handles: Mutex<Vec<AnyHandle>>,
    /// Querier-side correlation IDs whose reply path is sealed
    /// because the gateway emitted a synthetic `[0x03]` terminator
    /// on timeout. Reply / done callbacks built inside
    /// `spawn_query_with_timeout` consult this set BEFORE publishing
    /// any frame and drop the frame if the id is present. Entries are
    /// evicted by a delayed task after `effective_timeout`, so the
    /// set is bounded by "queries that timed out within the last
    /// eviction window." (Z5c)
    sealed_queries: Arc<Mutex<HashSet<QueryId>>>,
}

impl std::fmt::Debug for ZenohState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `QueryReplier` and `Arc<dyn CorrelatedPublish>` do not
        // implement Debug, so we skip the `correlation_map` and
        // `query_reply_publishers` fields here.
        f.debug_struct("ZenohState")
            .field("health", &self.health)
            .field("options", &self.options)
            .field("registry", &self.registry)
            .field("stop", &self.stop)
            .finish_non_exhaustive()
    }
}

impl ZenohState {
    /// Construct connector-internal state from configured options.
    ///
    /// The registry pre-allocates capacity sized to the sum of the
    /// bridge capacities (a sensible upper bound for channel count
    /// in steady state).
    #[must_use]
    pub fn new(options: ZenohConnectorOptions) -> Self {
        let cap = options
            .outbound_bridge_capacity
            .saturating_add(options.inbound_bridge_capacity);
        Self {
            health: Arc::new(ZenohHealthMonitor::new()),
            options,
            registry: Arc::new(Mutex::new(ChannelRegistry::with_capacity(cap))),
            stop: Arc::new(AtomicBool::new(false)),
            correlation_map: Arc::new(Mutex::new(HashMap::new())),
            query_reply_publishers: Arc::new(Mutex::new(HashMap::new())),
            handles: Mutex::new(Vec::new()),
            sealed_queries: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Take ownership of a handle whose drop releases a session-side
    /// resource (subscriber, queryable). Holds the handle for the
    /// lifetime of this state — when the connector drops, the bag
    /// drops, every handle's `Drop` impl runs.
    pub(crate) fn push_handle<H: Any + Send + Sync>(&self, h: H) {
        self.handles
            .lock()
            .expect("handles mutex not poisoned")
            .push(Box::new(h));
    }

    /// Borrow the shared health monitor.
    #[must_use]
    pub fn health(&self) -> &ZenohHealthMonitor {
        &self.health
    }

    /// Clone the `Arc<ZenohHealthMonitor>` for handoff to the
    /// dispatcher task.
    #[must_use]
    pub fn health_arc(&self) -> Arc<ZenohHealthMonitor> {
        Arc::clone(&self.health)
    }

    /// Borrow the configured options.
    #[must_use]
    pub const fn options(&self) -> &ZenohConnectorOptions {
        &self.options
    }

    /// Borrow the shared channel registry.
    #[must_use]
    pub const fn registry(&self) -> &Arc<Mutex<ChannelRegistry>> {
        &self.registry
    }

    /// Clone the dispatcher stop signal.
    #[must_use]
    pub fn stop_signal(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.stop)
    }

    /// Shared map of in-flight upstream queries — gateway-minted
    /// [`crate::registry::QueryId`] → [`crate::session::QueryReplier`]
    /// from the upstream session. Populated by `create_queryable`'s
    /// session callback; drained by the dispatcher in Z3f.
    #[must_use]
    pub(crate) fn correlation_map(&self) -> CorrelationMap {
        Arc::clone(&self.correlation_map)
    }

    /// Sidecar publisher map — descriptor name → `.reply.in`
    /// publisher. See Option B in the Z3 plan task 6: this lets the
    /// dispatcher's `QuerierOut` branch look up the matching reply
    /// publisher without re-entering the registry mutex.
    #[must_use]
    pub(crate) fn query_reply_publishers(&self) -> QueryReplyPublishers {
        Arc::clone(&self.query_reply_publishers)
    }

    /// Clone the sealed-queries sidecar handle. Used by
    /// `spawn_query_with_timeout` to insert a correlation id BEFORE
    /// publishing a synthetic `[0x03]` terminator, and consulted by
    /// the matching `on_reply` / `on_done` closures so any
    /// late-arriving upstream callback is silently dropped (`Z5c`).
    #[must_use]
    pub(crate) fn sealed_queries(&self) -> SealedQueries {
        Arc::clone(&self.sealed_queries)
    }
}

/// Plugin-side Zenoh connector.
///
/// Generic over a [`ZenohSessionLike`] back-end and a `PayloadCodec`.
/// Z2 Task 6 adds the `Connector` trait impl; this task lands the
/// struct + constructor only.
pub struct ZenohConnector<S, C>
where
    S: ZenohSessionLike,
{
    state: Arc<ZenohState>,
    codec: C,
    /// iceoryx2 node owned by the connector. Both the plugin-side
    /// publishers / subscribers and the gateway-side raw ports share
    /// this single node.
    node: Arc<Node<ipc::Service>>,
    /// Gateway-side tokio runtime owner.
    gateway: ZenohGateway,
    /// `Some(session)` until `register_with` consumes it (Z2 Task 6).
    session_slot: Mutex<Option<Arc<S>>>,
}

impl<S, C> ZenohConnector<S, C>
where
    S: ZenohSessionLike,
{
    /// Construct a plugin-side connector.
    ///
    /// Opens a fresh iceoryx2 node and a fresh tokio runtime (the
    /// gateway). The session is held until `Connector::register_with`
    /// is called (Z2 Task 6), at which point it's moved into the
    /// dispatcher task.
    ///
    /// # Errors
    /// Returns `ConnectorError::Stack` wrapping any iceoryx2 node
    /// creation or tokio runtime construction failure.
    pub fn new(
        state: Arc<ZenohState>,
        session: Arc<S>,
        codec: C,
    ) -> Result<Self, ConnectorError> {
        let node = NodeBuilder::new()
            .create::<ipc::Service>()
            .map_err(|e| ConnectorError::stack(NodeError(format!("{e:?}"))))?;
        let gateway = ZenohGateway::new(state.options().clone())
            .map_err(|e| ConnectorError::stack(NodeError(format!("gateway runtime: {e:?}"))))?;
        Ok(Self {
            state,
            codec,
            node: Arc::new(node),
            gateway,
            session_slot: Mutex::new(Some(session)),
        })
    }

    /// Borrow the shared state.
    #[must_use]
    pub const fn state(&self) -> &Arc<ZenohState> {
        &self.state
    }

    /// Borrow the iceoryx2 node (used by tests that need a second
    /// service-factory bound to the same node).
    #[must_use]
    pub const fn node(&self) -> &Arc<Node<ipc::Service>> {
        &self.node
    }

    /// Signal the dispatcher loop to exit. Tests use this for clean
    /// teardown before dropping the connector.
    pub fn stop_dispatcher(&self) {
        self.state.stop.store(true, Ordering::Release);
    }

    /// Internal: build an iceoryx2 [`ServiceFactory`] borrowing this
    /// connector's node. Used by `create_writer` / `create_reader`.
    pub(crate) fn factory(&self) -> ServiceFactory<'_> {
        ServiceFactory::new(&self.node)
    }

    /// Internal: take the session out of its slot (called once, by Z2
    /// Task 6's `register_with`).
    pub(crate) fn take_session(&self) -> Option<Arc<S>> {
        self.session_slot
            .lock()
            .expect("session slot mutex not poisoned")
            .take()
    }
}

// ── Connector trait impl ─────────────────────────────────────────────────────

use sonic_connector_core::{ChannelDescriptor, ConnectorHealth};
use sonic_connector_host::{Connector, HealthSubscription};
use sonic_connector_transport_iox::{ChannelReader, ChannelWriter};
use sonic_executor::{ControlFlow, Executor, item_with_triggers};

use crate::dispatcher::{IoxInboundPublish, IoxOutboundDrain, dispatcher_loop};
use crate::registry::{ChannelBinding, ChannelDirection, InboundPublish};
use crate::routing::ZenohRouting;

impl<S, C> Connector for ZenohConnector<S, C>
where
    S: ZenohSessionLike + 'static,
    C: sonic_connector_core::PayloadCodec + Clone + Send + Sync + 'static,
{
    type Routing = ZenohRouting;
    type Codec = C;

    fn name(&self) -> &'static str {
        "zenoh"
    }

    fn health(&self) -> ConnectorHealth {
        self.state.health().current()
    }

    fn subscribe_health(&self) -> HealthSubscription {
        HealthSubscription::new(self.state.health().subscribe())
    }

    fn register_with(&mut self, executor: &mut Executor) -> Result<(), ConnectorError> {
        // Move the session out — a second call sees `None` and returns
        // an error.
        let session = self
            .take_session()
            .ok_or_else(|| ConnectorError::stack(AlreadyRegistered))?;

        // Spawn the dispatcher loop on the gateway's tokio runtime
        // (REQ_0321).
        let handle = self
            .gateway
            .handle()
            .ok_or_else(|| ConnectorError::stack(GatewayShutDown))?;
        let registry = Arc::clone(self.state.registry());
        let stop = self.state.stop_signal();
        let tick = self.state.options().dispatcher_tick;
        let correlation_map = self.state.correlation_map();
        let query_reply_publishers = self.state.query_reply_publishers();
        let sealed_queries = self.state.sealed_queries();
        let query_timeout = self.state.options().query_timeout;

        // Z5b (REQ_0442): spawn the health watcher BEFORE the
        // dispatcher consumes `session`. Clone the Arcs the watcher
        // needs so the move below can take ownership of `session`.
        //
        // The watcher polls a combined (state, peer_count) observation
        // every 100ms and re-emits whenever either the state changes
        // or the peer count crosses the `min_peers` floor under
        // `Alive`. Captures `min_peers` from options into the closure.
        let session_for_health = Arc::clone(&session);
        let health = self.state.health_arc();
        let stop_for_health = Arc::clone(&stop);
        let min_peers = self.state.options().min_peers;
        handle.spawn(async move {
            let mut last_state = session_for_health.state();
            let mut last_peers = session_for_health.peer_count();
            tracing::info!(
                state = ?last_state,
                peers = last_peers,
                "zenoh health watcher started"
            );
            // Emit the initial observation so subscribers see the
            // starting (state, peer_count) combination.
            health.apply_observation(&last_state, last_peers, min_peers);
            while !stop_for_health.load(Ordering::Acquire) {
                tokio::time::sleep(Duration::from_millis(100)).await;
                let current_state = session_for_health.state();
                let current_peers = session_for_health.peer_count();
                // Re-emit only when something the mapping cares about
                // has changed: the state, or (under `Alive`) whether
                // the peer count is on the other side of the floor.
                let crossed_floor = match (&last_state, &current_state) {
                    (SessionState::Alive, SessionState::Alive) => min_peers
                        .is_some_and(|floor| {
                            (last_peers < floor) != (current_peers < floor)
                        }),
                    _ => false,
                };
                if current_state != last_state || crossed_floor {
                    tracing::info!(
                        from = ?last_state,
                        from_peers = last_peers,
                        to = ?current_state,
                        to_peers = current_peers,
                        "zenoh observation changed"
                    );
                    health.apply_observation(&current_state, current_peers, min_peers);
                    last_state = current_state.clone();
                    last_peers = current_peers;
                }
                if matches!(current_state, SessionState::Closed { .. }) {
                    // No further transitions are observable once the
                    // session is closed; exit so the runtime can shut
                    // down cleanly.
                    tracing::info!("zenoh session closed; health watcher exiting");
                    break;
                }
            }
        });

        handle.spawn(async move {
            let _ = dispatcher_loop(
                registry,
                session,
                stop,
                tick,
                correlation_map,
                query_reply_publishers,
                sealed_queries,
                query_timeout,
            )
            .await;
        });

        // Heartbeat `ExecutableItem` so the connector is a well-formed
        // `ConnectorHost` participant per REQ_0272. The dispatcher does
        // the real work; this item exists to satisfy the
        // executor-registration contract.
        let heartbeat = item_with_triggers(
            move |d| {
                d.interval(tick);
                Ok(())
            },
            |_ctx| Ok(ControlFlow::Continue),
        );
        executor.add(heartbeat).map_err(ConnectorError::stack)?;
        Ok(())
    }

    fn create_writer<T, const N: usize>(
        &self,
        descriptor: &ChannelDescriptor<Self::Routing, N>,
    ) -> Result<ChannelWriter<T, Self::Codec, N>, ConnectorError>
    where
        T: serde::Serialize,
    {
        let routing = descriptor.routing().clone();
        let svc_name = format!("{}.out", descriptor.name());
        let plugin_desc =
            ChannelDescriptor::<ZenohRouting, N>::new(svc_name.clone(), routing.clone())?;
        let factory = self.factory();

        // Plugin-side publisher (returned to caller).
        let writer = factory.create_writer::<T, _, _, N>(&plugin_desc, self.codec.clone())?;

        // Gateway-side raw subscriber — drains plugin's publishes into
        // `session.publish` on each dispatcher tick. Held behind `Arc`
        // (Z4a) so the async dispatcher can snapshot-clone the drain
        // out of the registry lock before awaiting on the session.
        let raw_reader = factory.create_raw_reader_named::<N>(&svc_name)?;
        let drain: Arc<dyn crate::registry::OutboundDrain> =
            Arc::new(IoxOutboundDrain::<N>::new(raw_reader));

        self.state
            .registry()
            .lock()
            .expect("registry mutex not poisoned")
            .register(
                descriptor.name().to_string(),
                routing,
                ChannelDirection::Outbound,
                ChannelBinding::Outbound(drain),
            )?;
        Ok(writer)
    }

    fn create_reader<T, const N: usize>(
        &self,
        descriptor: &ChannelDescriptor<Self::Routing, N>,
    ) -> Result<ChannelReader<T, Self::Codec, N>, ConnectorError>
    where
        T: serde::de::DeserializeOwned,
    {
        let routing = descriptor.routing().clone();
        let svc_name = format!("{}.in", descriptor.name());
        let plugin_desc =
            ChannelDescriptor::<ZenohRouting, N>::new(svc_name.clone(), routing.clone())?;
        let factory = self.factory();

        // Plugin-side subscriber (returned to caller).
        let reader = factory.create_reader::<T, _, _, N>(&plugin_desc, self.codec.clone())?;

        // Gateway-side raw publisher — written to from session
        // subscribe callbacks so Zenoh-delivered bytes land on the
        // plugin's iox subscriber.
        let raw_writer = factory.create_raw_writer_named::<N>(&svc_name)?;
        let inbound = Arc::new(IoxInboundPublish::<N>::new(raw_writer));
        let inbound_for_callback = Arc::clone(&inbound);

        // Wire the session subscriber: bytes from Zenoh arrive in the
        // callback and are forwarded to the iox raw publisher.
        let sink: crate::session::PayloadSink = Box::new(move |bytes: &[u8]| {
            let _ = inbound_for_callback.publish_bytes(bytes);
        });

        // Z4a: `subscribe` is async. Snapshot the session out of the
        // slot, drop the std::sync::Mutex guard, then bridge to the
        // gateway's tokio runtime via `Handle::block_on`. CRITICAL:
        // the guard MUST NOT be held across the block_on call — std
        // Mutexes cannot be held across `.await`s, and `block_on`
        // counts.
        let sub_handle = {
            let session = {
                let guard = self
                    .session_slot
                    .lock()
                    .expect("session slot mutex not poisoned");
                guard
                    .as_ref()
                    .ok_or_else(|| ConnectorError::stack(SessionAlreadyTaken))?
                    .clone()
            };
            let handle = self
                .gateway
                .handle()
                .ok_or_else(|| ConnectorError::stack(GatewayShutDown))?;
            handle
                .block_on(session.subscribe(&routing, sink))
                .map_err(|e| ConnectorError::stack(SessionFailure(format!("{e}"))))?
        };
        // Z4d: stash the handle in the state's typed-erased bag.
        // When the connector drops, the bag drops, this handle's
        // `Drop` impl runs and tears down the session subscriber.
        self.state.push_handle(sub_handle);

        self.state
            .registry()
            .lock()
            .expect("registry mutex not poisoned")
            .register(
                descriptor.name().to_string(),
                routing,
                ChannelDirection::Inbound,
                ChannelBinding::Inbound(Box::new(IoxInboundPublishOwned::new(inbound))),
            )?;

        Ok(reader)
    }
}

/// Owning wrapper around `Arc<IoxInboundPublish<N>>` so the registry
/// can hold the publisher behind `Box<dyn InboundPublish>` while the
/// session callback also holds a clone of the same `Arc`.
struct IoxInboundPublishOwned<const N: usize> {
    inner: Arc<IoxInboundPublish<N>>,
}

impl<const N: usize> IoxInboundPublishOwned<N> {
    const fn new(inner: Arc<IoxInboundPublish<N>>) -> Self {
        Self { inner }
    }
}

impl<const N: usize> InboundPublish for IoxInboundPublishOwned<N> {
    fn publish_bytes(&self, bytes: &[u8]) -> Result<(), ConnectorError> {
        self.inner.publish_bytes(bytes)
    }
}

// ── Query-side non-trait methods ─────────────────────────────────────────────
//
// Per `ADR_0040` / `REQ_0290` / `REQ_0294` — queries are NOT on the
// `Connector` trait. They're concrete methods on the Zenoh-specific
// connector type.

use crate::dispatcher::{IoxCorrelatedPublish, IoxQuerierDrain, IoxReplyDrain};
use crate::querier::ZenohQuerier;
use crate::queryable::ZenohQueryable;
use crate::registry::{CorrelatedPublish, QuerierDrain, ReplyDrain};

impl<S, C> ZenohConnector<S, C>
where
    S: ZenohSessionLike + 'static,
    C: sonic_connector_core::PayloadCodec + Clone + Send + Sync + 'static,
{
    /// Open a query channel and return a [`ZenohQuerier<Q, R, C, N>`].
    ///
    /// `Q` is the request type; `R` is the reply type. The querier
    /// uses the connector's `default_timeout` (`REQ_0425`).
    ///
    /// Creates two iceoryx2 services:
    ///
    /// * `{name}.query.out` — plugin writes encoded `Q` here; gateway
    ///   drains and calls `session.query`.
    /// * `{name}.reply.in` — gateway publishes framed reply bytes
    ///   here; plugin's querier reads them.
    ///
    /// # Errors
    /// Returns [`ConnectorError`] on iox failure or registry conflict.
    ///
    /// # Panics
    /// Panics only if the registry mutex is poisoned, which would
    /// require another thread to panic while holding the lock.
    pub fn create_querier<Q, R, const N: usize>(
        &self,
        descriptor: &ChannelDescriptor<ZenohRouting, N>,
    ) -> Result<ZenohQuerier<Q, R, C, N>, ConnectorError>
    where
        Q: serde::Serialize,
        R: serde::de::DeserializeOwned,
    {
        let routing = descriptor.routing().clone();
        let factory = self.factory();
        let q_name = format!("{}.query.out", descriptor.name());
        let r_name = format!("{}.reply.in", descriptor.name());

        // Plugin-side: writes encoded Q, reads framed replies.
        let q_writer_plugin = factory.create_raw_writer_named::<N>(&q_name)?;
        let r_reader_plugin = factory.create_raw_reader_named::<N>(&r_name)?;

        // Gateway-side: drains plugin Q-bytes, publishes framed replies.
        let q_reader_gw = factory.create_raw_reader_named::<N>(&q_name)?;
        let r_writer_gw = factory.create_raw_writer_named::<N>(&r_name)?;

        let q_drain: Arc<dyn QuerierDrain> =
            Arc::new(IoxQuerierDrain::<N>::new(q_reader_gw));

        // Wrap the `.reply.in` publisher in an `Arc` so the registry
        // binding and the sidecar map can share it. The dispatcher's
        // `QuerierOut` branch looks the publisher up in the sidecar
        // map by descriptor name and binds it into the reply-stamping
        // callbacks for `session.query`.
        let r_publish_arc: Arc<IoxCorrelatedPublish<N>> =
            Arc::new(IoxCorrelatedPublish::<N>::new(r_writer_gw));

        self.state
            .query_reply_publishers()
            .lock()
            .expect("query reply publishers mutex not poisoned")
            .insert(
                descriptor.name().to_string(),
                Arc::clone(&r_publish_arc) as Arc<dyn CorrelatedPublish>,
            );

        let r_publish: Box<dyn CorrelatedPublish> =
            Box::new(IoxCorrelatedPublishOwned::new(r_publish_arc));

        {
            let mut reg = self
                .state
                .registry()
                .lock()
                .expect("registry mutex not poisoned");
            reg.register(
                descriptor.name().to_string(),
                routing.clone(),
                ChannelDirection::QuerierOut,
                ChannelBinding::QuerierOut(q_drain),
            )?;
            reg.register(
                descriptor.name().to_string(),
                routing,
                ChannelDirection::QuerierReplyIn,
                ChannelBinding::QuerierReplyIn(r_publish),
            )?;
        }

        Ok(ZenohQuerier::<Q, R, C, N>::new(
            q_writer_plugin,
            r_reader_plugin,
            self.codec.clone(),
        ))
    }

    /// Open a query channel and return a
    /// [`ZenohQueryable<Q, R, C, N>`].
    ///
    /// The connector's session is registered with a queryable callback
    /// that funnels upstream queries to `{name}.query.in`; replies sent
    /// via the returned handle land on `{name}.reply.out` and are
    /// routed by the gateway dispatcher (Z3f wires the routing).
    ///
    /// # Errors
    /// Returns [`ConnectorError`] on iox failure, registry conflict, or
    /// session-subscribe failure.
    ///
    /// # Panics
    /// Panics only if the registry or session-slot mutexes are
    /// poisoned, which would require another thread to panic while
    /// holding either lock.
    pub fn create_queryable<Q, R, const N: usize>(
        &self,
        descriptor: &ChannelDescriptor<ZenohRouting, N>,
    ) -> Result<ZenohQueryable<Q, R, C, N>, ConnectorError>
    where
        Q: serde::de::DeserializeOwned,
        R: serde::Serialize,
    {
        let routing = descriptor.routing().clone();
        let factory = self.factory();
        let q_name = format!("{}.query.in", descriptor.name());
        let r_name = format!("{}.reply.out", descriptor.name());

        // Plugin-side: reads incoming queries, writes framed replies.
        let q_reader_plugin = factory.create_raw_reader_named::<N>(&q_name)?;
        let r_writer_plugin = factory.create_raw_writer_named::<N>(&r_name)?;

        // Gateway-side: publishes incoming Q-bytes, drains framed replies.
        let q_writer_gw = factory.create_raw_writer_named::<N>(&q_name)?;
        let r_reader_gw = factory.create_raw_reader_named::<N>(&r_name)?;

        // Wrap the q_writer_gw in an Arc so the session callback and
        // the registry binding can share it.
        let q_publish: Arc<IoxCorrelatedPublish<N>> =
            Arc::new(IoxCorrelatedPublish::<N>::new(q_writer_gw));
        let q_publish_for_callback = Arc::clone(&q_publish);
        let r_drain: Arc<dyn ReplyDrain> =
            Arc::new(IoxReplyDrain::<N>::new(r_reader_gw));

        // Wire the session's declare_queryable callback. On each
        // upstream query: mint a QueryId, stash the replier in the
        // correlation map, publish the request bytes on .query.in
        // with that QueryId. The dispatcher (Z3f) drains .reply.out
        // and uses the correlation_map to forward replies to the
        // stashed QueryReplier.
        let correlation_map = self.state.correlation_map();
        let on_query: crate::session::QuerySink = Box::new(
            move |req: &[u8], replier: crate::session::QueryReplier| {
                let id = crate::querier::mint_query_id();
                correlation_map
                    .lock()
                    .expect("correlation map mutex not poisoned")
                    .insert(id, replier);
                // Publish the request bytes on .query.in with the
                // minted QueryId as correlation_id. Drop publish
                // errors silently — the plugin will time out anyway
                // (REQ_0425, handled in Z3f).
                let _ = q_publish_for_callback.publish_with_correlation(id, req);
            },
        );

        // Subscribe with the session. The session is consumed by
        // register_with — if it's already gone, we can't declare.
        //
        // Z4a: `declare_queryable` is async. Same `block_on` bridge
        // pattern as `create_reader` — snapshot the session out of
        // the slot under the std::sync::Mutex guard, drop the guard,
        // then `block_on` on the gateway runtime.
        let qable_handle = {
            let session = {
                let guard = self
                    .session_slot
                    .lock()
                    .expect("session slot mutex not poisoned");
                guard
                    .as_ref()
                    .ok_or_else(|| ConnectorError::stack(SessionAlreadyTaken))?
                    .clone()
            };
            let handle = self
                .gateway
                .handle()
                .ok_or_else(|| ConnectorError::stack(GatewayShutDown))?;
            handle
                .block_on(session.declare_queryable(&routing, on_query))
                .map_err(|e| {
                    ConnectorError::stack(SessionFailure(format!("declare_queryable: {e}")))
                })?
        };
        // Z4d: stash the handle in the state's typed-erased bag,
        // mirroring `create_reader`. Connector drop → bag drop →
        // handle drop → session queryable torn down.
        self.state.push_handle(qable_handle);

        {
            let mut reg = self
                .state
                .registry()
                .lock()
                .expect("registry mutex not poisoned");
            reg.register(
                descriptor.name().to_string(),
                routing.clone(),
                ChannelDirection::QueryableQueryIn,
                ChannelBinding::QueryableQueryIn(Box::new(IoxCorrelatedPublishOwned::new(
                    q_publish,
                ))),
            )?;
            reg.register(
                descriptor.name().to_string(),
                routing,
                ChannelDirection::QueryableReplyOut,
                ChannelBinding::QueryableReplyOut(r_drain),
            )?;
        }

        Ok(ZenohQueryable::<Q, R, C, N>::new(
            q_reader_plugin,
            r_writer_plugin,
            self.codec.clone(),
        ))
    }
}

/// Owning wrapper around `Arc<IoxCorrelatedPublish<N>>` so the registry
/// can hold the publisher behind `Box<dyn CorrelatedPublish>` while
/// the session callback also holds a clone.
struct IoxCorrelatedPublishOwned<const N: usize> {
    inner: Arc<IoxCorrelatedPublish<N>>,
}

impl<const N: usize> IoxCorrelatedPublishOwned<N> {
    const fn new(inner: Arc<IoxCorrelatedPublish<N>>) -> Self {
        Self { inner }
    }
}

impl<const N: usize> CorrelatedPublish for IoxCorrelatedPublishOwned<N> {
    fn publish_with_correlation(
        &self,
        id: QueryId,
        bytes: &[u8],
    ) -> Result<(), ConnectorError> {
        self.inner.publish_with_correlation(id, bytes)
    }
}

#[derive(Debug)]
struct NodeError(String);

impl core::fmt::Display for NodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NodeError {}

#[derive(Debug)]
struct AlreadyRegistered;

impl core::fmt::Display for AlreadyRegistered {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("zenoh connector: register_with already called; session was moved into dispatcher")
    }
}

impl std::error::Error for AlreadyRegistered {}

#[derive(Debug)]
struct GatewayShutDown;

impl core::fmt::Display for GatewayShutDown {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("zenoh connector: gateway runtime has shut down")
    }
}

impl std::error::Error for GatewayShutDown {}

#[derive(Debug)]
struct SessionAlreadyTaken;

impl core::fmt::Display for SessionAlreadyTaken {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("zenoh connector: session has already been consumed by register_with")
    }
}

impl std::error::Error for SessionAlreadyTaken {}

#[derive(Debug)]
struct SessionFailure(String);

impl core::fmt::Display for SessionFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SessionFailure {}
