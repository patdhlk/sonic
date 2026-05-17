//! [`CanConnector`] — plugin-side implementation of the framework's
//! [`Connector`] trait. `REQ_0600`.
//!
//! Owns:
//!
//! * An iceoryx2 [`Node`] for opening pub/sub services. Plugin
//!   publishers and gateway subscribers share this node.
//! * A `Vec<I>` of pre-constructed driver instances, one per iface
//!   configured in [`CanConnectorOptions::ifaces`]. Drivers are
//!   moved into per-iface dispatcher tasks on `register_with`.
//! * A [`CanGateway`] holding the gateway's tokio runtime.
//! * A shared [`CanState`] containing the health monitor, the
//!   channel registry, and the dispatcher stop signal.
//!
//! On `create_writer` / `create_reader` (`REQ_0223`):
//!
//! 1. Validate the descriptor's routing iface is one configured on
//!    this gateway (`REQ_0521`).
//! 2. Validate the channel's `N` const generic equals the routing
//!    kind's max payload (`REQ_0612`).
//! 3. Open the plugin-side iceoryx2 service `"{name}.out"` / `.in`.
//! 4. Open the paired gateway-side raw port on the same service.
//! 5. Register the channel on the shared [`ChannelRegistry`].
//!
//! On `register_with`:
//!
//! * Take the driver vec out of the connector and spawn one
//!   [`dispatcher_loop`] task per iface (`REQ_0505`). Also register
//!   a heartbeat executor item to satisfy `REQ_0272`.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use iceoryx2::node::Node;
use iceoryx2::prelude::{NodeBuilder, ipc};
use sonic_connector_core::{ChannelDescriptor, ConnectorError, ConnectorHealth};
use sonic_connector_host::{Connector, HealthSubscription};
use sonic_connector_transport_iox::{ChannelReader, ChannelWriter, ServiceFactory};
use sonic_executor::{ControlFlow, Executor, item_with_triggers};

use crate::dispatcher::{DEFAULT_TX_TICK, IoxInboundPublish, IoxOutboundDrain, dispatcher_loop};
use crate::driver::CanInterfaceLike;
use crate::gateway::CanGateway;
use crate::health::CanHealthMonitor;
use crate::options::CanConnectorOptions;
use crate::registry::{ChannelBinding, ChannelRegistry, Direction};
use crate::routing::CanRouting;

/// Plugin-side CAN connector.
///
/// Generic over a [`CanInterfaceLike`] driver type (typically
/// [`crate::MockCanInterface`] in layer-1 tests, `RealCanInterface`
/// in layer-2) and a `PayloadCodec` (`REQ_0211`).
pub struct CanConnector<I, C>
where
    I: CanInterfaceLike,
{
    state: Arc<CanState>,
    codec: C,
    node: Arc<Node<ipc::Service>>,
    gateway: CanGateway,
    /// `Some(drivers)` until `register_with` consumes them.
    /// Length equals `options.ifaces().len()`; index `i` is the
    /// driver for `options.ifaces()[i]`.
    drivers_slot: Mutex<Option<Vec<I>>>,
    tx_tick: Duration,
}

/// Connector-internal state shared between [`CanConnector`] and the
/// per-iface dispatcher tasks.
#[derive(Debug)]
pub struct CanState {
    options: CanConnectorOptions,
    health: Arc<CanHealthMonitor>,
    registry: Arc<Mutex<ChannelRegistry>>,
    stop: Arc<AtomicBool>,
}

impl CanState {
    /// Construct connector-internal state from configured options.
    #[must_use]
    pub fn new(options: CanConnectorOptions) -> Self {
        let outbound_cap = options.outbound_capacity();
        let inbound_cap = options.inbound_capacity();
        let capacity = outbound_cap.saturating_add(inbound_cap);
        let health = Arc::new(CanHealthMonitor::new(options.ifaces()));
        Self {
            options,
            health,
            registry: Arc::new(Mutex::new(ChannelRegistry::with_capacity(capacity))),
            stop: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Borrow the shared health monitor.
    #[must_use]
    pub fn health(&self) -> &CanHealthMonitor {
        &self.health
    }

    /// Borrow the configured options.
    #[must_use]
    pub const fn options(&self) -> &CanConnectorOptions {
        &self.options
    }

    /// Borrow the shared channel registry.
    #[must_use]
    pub const fn registry(&self) -> &Arc<Mutex<ChannelRegistry>> {
        &self.registry
    }
}

impl<I, C> CanConnector<I, C>
where
    I: CanInterfaceLike,
{
    /// Construct a plugin-side connector with pre-built driver
    /// instances. `drivers[i]` must be a driver bound to
    /// `state.options().ifaces()[i]`.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectorError::Stack`] wrapping any iceoryx2 node
    /// creation failure or tokio runtime construction failure.
    /// Returns [`ConnectorError::InvalidDescriptor`] when
    /// `drivers.len() != options.ifaces().len()`.
    pub fn new(state: Arc<CanState>, drivers: Vec<I>, codec: C) -> Result<Self, ConnectorError> {
        if drivers.len() != state.options().ifaces().len() {
            return Err(ConnectorError::InvalidDescriptor(format!(
                "CanConnector: drivers.len() {} does not match ifaces.len() {}",
                drivers.len(),
                state.options().ifaces().len()
            )));
        }
        let node = NodeBuilder::new()
            .create::<ipc::Service>()
            .map_err(|e| ConnectorError::stack(NodeError(format!("{e:?}"))))?;
        let gateway = CanGateway::new(state.options().clone())
            .map_err(|e| ConnectorError::stack(NodeError(format!("gateway runtime: {e:?}"))))?;
        Ok(Self {
            state,
            codec,
            node: Arc::new(node),
            gateway,
            drivers_slot: Mutex::new(Some(drivers)),
            tx_tick: DEFAULT_TX_TICK,
        })
    }

    /// Override the per-iteration TX drain tick.
    #[must_use]
    pub const fn with_tx_tick(mut self, tick: Duration) -> Self {
        self.tx_tick = tick;
        self
    }

    /// Borrow the shared state.
    #[must_use]
    pub const fn state(&self) -> &Arc<CanState> {
        &self.state
    }

    /// Signal every dispatcher task to exit. Used in tests that want
    /// a clean teardown before dropping the connector.
    pub fn stop_dispatchers(&self) {
        self.state.stop.store(true, Ordering::Release);
    }

    /// Internal: build an iceoryx2 [`ServiceFactory`] borrowing this
    /// connector's node.
    fn factory(&self) -> ServiceFactory<'_> {
        ServiceFactory::new(&self.node)
    }

    fn iface_is_configured(&self, iface: &crate::routing::CanIface) -> bool {
        self.state.options().ifaces().contains(iface)
    }
}

impl<I, C> Connector for CanConnector<I, C>
where
    I: CanInterfaceLike,
    C: sonic_connector_core::PayloadCodec + Clone + Send + 'static,
{
    type Routing = CanRouting;
    type Codec = C;

    fn name(&self) -> &str {
        "can"
    }

    fn health(&self) -> ConnectorHealth {
        self.state.health.current()
    }

    fn subscribe_health(&self) -> HealthSubscription {
        HealthSubscription::new(self.state.health.subscribe())
    }

    fn register_with(&mut self, executor: &mut Executor) -> Result<(), ConnectorError> {
        let drivers = self
            .drivers_slot
            .lock()
            .expect("drivers slot mutex not poisoned")
            .take()
            .ok_or_else(|| ConnectorError::stack(AlreadyRegistered))?;

        let handle = self
            .gateway
            .handle()
            .ok_or_else(|| ConnectorError::stack(GatewayShutDown))?;

        let ifaces = self.state.options().ifaces().to_vec();
        for (iface, driver) in ifaces.into_iter().zip(drivers.into_iter()) {
            let registry = Arc::clone(self.state.registry());
            let health = Arc::clone(&self.state.health);
            let stop = Arc::clone(&self.state.stop);
            let policy = self.state.options().new_reconnect_policy();
            let tick = self.tx_tick;
            handle.spawn(async move {
                let _ = dispatcher_loop(iface, driver, registry, health, policy, stop, tick).await;
            });
        }

        // Heartbeat ExecutableItem so the connector is a well-formed
        // ConnectorHost participant per REQ_0272. Each dispatcher
        // task does the real work; this item exists to satisfy the
        // executor-registration contract.
        let tick = self.tx_tick.max(Duration::from_millis(1));
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
        let routing = *descriptor.routing();
        validate_routing::<N>(&routing, self)?;
        let svc_name = service_name(descriptor.name(), Direction::Outbound);
        let plugin_desc = ChannelDescriptor::<CanRouting, N>::new(svc_name.clone(), routing)?;
        let factory = self.factory();
        let writer = factory.create_writer::<T, _, _, N>(&plugin_desc, self.codec.clone())?;
        let raw_reader = factory.create_raw_reader_named::<N>(&svc_name)?;
        let drain: Box<dyn crate::OutboundDrain> = Box::new(IoxOutboundDrain::<N>::new(raw_reader));
        self.state
            .registry()
            .lock()
            .expect("registry mutex not poisoned")
            .register(
                descriptor.name().to_string(),
                routing,
                Direction::Outbound,
                ChannelBinding::Outbound(drain),
            );
        Ok(writer)
    }

    fn create_reader<T, const N: usize>(
        &self,
        descriptor: &ChannelDescriptor<Self::Routing, N>,
    ) -> Result<ChannelReader<T, Self::Codec, N>, ConnectorError>
    where
        T: serde::de::DeserializeOwned,
    {
        let routing = *descriptor.routing();
        validate_routing::<N>(&routing, self)?;
        let svc_name = service_name(descriptor.name(), Direction::Inbound);
        let plugin_desc = ChannelDescriptor::<CanRouting, N>::new(svc_name.clone(), routing)?;
        let factory = self.factory();
        let reader = factory.create_reader::<T, _, _, N>(&plugin_desc, self.codec.clone())?;
        let raw_writer = factory.create_raw_writer_named::<N>(&svc_name)?;
        let publish: Box<dyn crate::InboundPublish> =
            Box::new(IoxInboundPublish::<N>::new(raw_writer));
        self.state
            .registry()
            .lock()
            .expect("registry mutex not poisoned")
            .register(
                descriptor.name().to_string(),
                routing,
                Direction::Inbound,
                ChannelBinding::Inbound(publish),
            );
        Ok(reader)
    }
}

fn validate_routing<const N: usize>(
    routing: &CanRouting,
    connector: &CanConnector<impl CanInterfaceLike, impl Send>,
) -> Result<(), ConnectorError> {
    if !connector.iface_is_configured(&routing.iface) {
        return Err(ConnectorError::InvalidDescriptor(format!(
            "CanRouting::iface {} not in CanConnectorOptions::ifaces",
            routing.iface
        )));
    }
    let expected = routing.kind.max_payload();
    if N != expected {
        return Err(ConnectorError::InvalidDescriptor(format!(
            "ChannelDescriptor max_payload_size {N} does not match CanFrameKind::{:?}.max_payload() = {expected}",
            routing.kind
        )));
    }
    Ok(())
}

fn service_name(base: &str, direction: Direction) -> String {
    match direction {
        Direction::Outbound => format!("{base}.out"),
        Direction::Inbound => format!("{base}.in"),
    }
}

#[derive(Debug)]
struct NodeError(String);

impl core::fmt::Display for NodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "iceoryx2 node: {}", self.0)
    }
}

impl std::error::Error for NodeError {}

#[derive(Debug)]
struct AlreadyRegistered;

impl core::fmt::Display for AlreadyRegistered {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "can connector: register_with already called; drivers were moved into dispatcher"
        )
    }
}

impl std::error::Error for AlreadyRegistered {}

#[derive(Debug)]
struct GatewayShutDown;

impl core::fmt::Display for GatewayShutDown {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "can connector: gateway runtime is shut down")
    }
}

impl std::error::Error for GatewayShutDown {}
