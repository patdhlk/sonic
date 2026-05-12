//! [`ZenohConnector`] — plugin-side implementation of the framework's
//! `Connector` trait (`REQ_0400`).
//!
//! Generic over a [`ZenohSessionLike`] back-end (the session — mock
//! in tests, real in Z4) and a `PayloadCodec` (`REQ_0211`).
//!
//! Z2 Task 5 lands the struct + constructor; Z2 Task 6 adds the full
//! `Connector` trait impl (`name`, `health`, `subscribe_health`,
//! `register_with`, `create_writer`, `create_reader`).

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};

use iceoryx2::node::Node;
use iceoryx2::prelude::{NodeBuilder, ipc};
use sonic_connector_core::ConnectorError;
use sonic_connector_transport_iox::ServiceFactory;

use crate::gateway::ZenohGateway;
use crate::health::ZenohHealthMonitor;
use crate::options::ZenohConnectorOptions;
use crate::registry::ChannelRegistry;
use crate::session::ZenohSessionLike;

/// Connector-internal state shared between [`ZenohConnector`] and the
/// gateway-side dispatcher.
#[derive(Debug)]
pub struct ZenohState {
    health: Arc<ZenohHealthMonitor>,
    options: ZenohConnectorOptions,
    registry: Arc<Mutex<ChannelRegistry>>,
    stop: Arc<AtomicBool>,
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
        }
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
        handle.spawn(async move {
            let _ = dispatcher_loop(registry, session, stop, tick).await;
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
        // `session.publish` on each dispatcher tick.
        let raw_reader = factory.create_raw_reader_named::<N>(&svc_name)?;
        let drain: Box<dyn crate::registry::OutboundDrain> =
            Box::new(IoxOutboundDrain::<N>::new(raw_reader));

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
        //
        // TODO(Z4): track the returned `SubscriptionHandle` rather than
        // leaking it; Z4 adds explicit channel lifecycle management.
        let sink: crate::session::PayloadSink = Box::new(move |bytes: &[u8]| {
            let _ = inbound_for_callback.publish_bytes(bytes);
        });

        let sub_handle = self
            .session_slot
            .lock()
            .expect("session slot mutex not poisoned")
            .as_ref()
            .ok_or_else(|| ConnectorError::stack(SessionAlreadyTaken))?
            .subscribe(&routing, sink)
            .map_err(|e| ConnectorError::stack(SessionFailure(format!("{e}"))))?;
        // Intentionally leak the handle for the lifetime of the
        // connector; lifecycle cleanup is deferred to Z4.
        Box::leak(Box::new(sub_handle));

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
