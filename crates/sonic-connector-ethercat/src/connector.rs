//! [`EthercatConnector`] — plugin-side implementation of the
//! framework's [`Connector`] trait. `REQ_0310`.
//!
//! Owns:
//!
//! * An iceoryx2 [`Node`] for opening pub/sub services. The plugin's
//!   publishers and the gateway's subscribers share this node — they
//!   live in the same process per `ADR_0028`.
//! * A configured [`BusDriver`] (`D`). The driver is moved into the
//!   dispatcher task when `register_with` is called and driven on the
//!   gateway's tokio runtime thereafter.
//! * An [`EthercatGateway`] holding the gateway's tokio runtime.
//! * A shared [`EthercatState`] containing the health monitor, the
//!   per-channel routing registry (`REQ_0328`), and the dispatcher's
//!   stop signal.
//!
//! On `create_writer` / `create_reader` (`REQ_0223`):
//!
//! 1. Open the plugin-side iceoryx2 service named
//!    `"{descriptor.name()}.out"` (for writers) or `".in"` (readers).
//! 2. Open the paired gateway-side iceoryx2 port on the same service.
//! 3. Register the channel on the shared [`ChannelRegistry`].
//!
//! On `register_with`:
//!
//! * Take the driver out of the connector and spawn the dispatcher
//!   loop on the gateway's tokio runtime (`REQ_0321`). Also register
//!   a heartbeat executor item to satisfy `REQ_0272`.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use iceoryx2::node::Node;
use iceoryx2::prelude::{NodeBuilder, ipc};
use sonic_connector_core::{ChannelDescriptor, ConnectorError, ConnectorHealth};
use sonic_connector_host::{Connector, HealthSubscription};
use sonic_connector_transport_iox::{ChannelReader, ChannelWriter, ServiceFactory};
use sonic_executor::{ControlFlow, Executor, item_with_triggers};

use crate::dispatcher::{IoxInboundPublish, IoxOutboundDrain, dispatcher_loop};
use crate::driver::BusDriver;
use crate::gateway::EthercatGateway;
use crate::health::EthercatHealthMonitor;
use crate::options::EthercatConnectorOptions;
use crate::registry::{ChannelBinding, ChannelRegistry};
use crate::routing::{EthercatRouting, PdoDirection};
use crate::runner::CycleRunner;

/// Plugin-side `EtherCAT` connector.
///
/// Generic over a [`BusDriver`] (the gateway-side cycle driver —
/// typically `MockBusDriver` in tests or `EthercatBusDriver` in
/// production) and a `PayloadCodec` (`REQ_0211`).
pub struct EthercatConnector<D, C>
where
    D: BusDriver,
{
    state: Arc<EthercatState>,
    codec: C,
    /// iceoryx2 node owned by the connector. Both the plugin-side
    /// publishers / subscribers and the gateway-side raw ports share
    /// this single node, which iceoryx2 supports natively for
    /// in-process pub/sub.
    node: Arc<Node<ipc::Service>>,
    /// Gateway-side tokio runtime owner. The dispatcher loop runs on
    /// this runtime once [`Self::register_with`] is called.
    gateway: EthercatGateway,
    /// `Some(driver)` until [`Self::register_with`] consumes it.
    driver_slot: Mutex<Option<D>>,
}

/// Connector-internal state shared between [`EthercatConnector`] and
/// the gateway-side dispatcher. Both sides live in the same process
/// per [ADR_0028](../../spec/architecture/connector.rst).
#[derive(Debug)]
pub struct EthercatState {
    /// Health monitor — wrapped in `Arc` so the dispatcher's
    /// [`CycleRunner`] and the plugin's `subscribe_health` see the
    /// same transitions.
    health: Arc<EthercatHealthMonitor>,
    /// Configured options. Cloned into the dispatcher task.
    options: EthercatConnectorOptions,
    /// Channel registry populated by [`EthercatConnector::create_writer`]
    /// / [`EthercatConnector::create_reader`] and iterated each cycle
    /// by the dispatcher (`REQ_0328`).
    registry: Arc<Mutex<ChannelRegistry>>,
    /// Atomic flag the dispatcher loop polls each iteration; set
    /// `true` by [`EthercatConnector::stop_dispatcher`] to ask the
    /// loop to exit.
    stop: Arc<AtomicBool>,
}

impl EthercatState {
    /// Construct connector-internal state from configured options.
    /// The registry starts empty; channels are appended via
    /// [`EthercatConnector::create_writer`] / `create_reader`.
    #[must_use]
    pub fn new(options: EthercatConnectorOptions) -> Self {
        let outbound_cap = options.outbound_capacity();
        let inbound_cap = options.inbound_capacity();
        let capacity = outbound_cap.saturating_add(inbound_cap);
        Self {
            health: Arc::new(EthercatHealthMonitor::new()),
            options,
            registry: Arc::new(Mutex::new(ChannelRegistry::with_capacity(capacity))),
            stop: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Borrow the shared health monitor.
    #[must_use]
    pub fn health(&self) -> &EthercatHealthMonitor {
        &self.health
    }

    /// Clone the `Arc<EthercatHealthMonitor>` for handoff to the
    /// dispatcher task (`CycleRunner::new` takes an `Arc`).
    #[must_use]
    pub fn health_arc(&self) -> Arc<EthercatHealthMonitor> {
        Arc::clone(&self.health)
    }

    /// Borrow the configured options.
    #[must_use]
    pub const fn options(&self) -> &EthercatConnectorOptions {
        &self.options
    }

    /// Borrow the shared channel registry.
    #[must_use]
    pub const fn registry(&self) -> &Arc<Mutex<ChannelRegistry>> {
        &self.registry
    }

    /// Clone the dispatcher stop signal. Set to `true` to ask the
    /// dispatcher loop to exit.
    #[must_use]
    pub fn stop_signal(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.stop)
    }
}

impl<D, C> EthercatConnector<D, C>
where
    D: BusDriver,
{
    /// Construct a plugin-side connector with a configured driver.
    /// Opens a fresh iceoryx2 node and a fresh tokio runtime.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectorError::Stack`] wrapping any iceoryx2 node
    /// creation failure or tokio runtime construction failure.
    pub fn new(state: Arc<EthercatState>, driver: D, codec: C) -> Result<Self, ConnectorError> {
        let node = NodeBuilder::new()
            .create::<ipc::Service>()
            .map_err(|e| ConnectorError::stack(NodeError(format!("{e:?}"))))?;
        let gateway = EthercatGateway::new(state.options().clone())
            .map_err(|e| ConnectorError::stack(NodeError(format!("gateway runtime: {e:?}"))))?;
        Ok(Self {
            state,
            codec,
            node: Arc::new(node),
            gateway,
            driver_slot: Mutex::new(Some(driver)),
        })
    }

    /// Borrow the shared state.
    #[must_use]
    pub const fn state(&self) -> &Arc<EthercatState> {
        &self.state
    }

    /// Signal the dispatcher loop to exit. Used in tests that want a
    /// clean teardown before dropping the connector.
    pub fn stop_dispatcher(&self) {
        self.state.stop.store(true, Ordering::Release);
    }

    /// Internal: build an iceoryx2 [`ServiceFactory`] borrowing this
    /// connector's node. Used by both the plugin-side and
    /// gateway-side port creation.
    fn factory(&self) -> ServiceFactory<'_> {
        ServiceFactory::new(&self.node)
    }
}

impl<D, C> Connector for EthercatConnector<D, C>
where
    D: BusDriver,
    C: sonic_connector_core::PayloadCodec + Clone + Send + 'static,
{
    type Routing = EthercatRouting;
    type Codec = C;

    fn name(&self) -> &'static str {
        "ethercat"
    }

    fn health(&self) -> ConnectorHealth {
        self.state.health.current()
    }

    fn subscribe_health(&self) -> HealthSubscription {
        HealthSubscription::new(self.state.health.subscribe())
    }

    fn register_with(&mut self, executor: &mut Executor) -> Result<(), ConnectorError> {
        // Move the driver out — register_with is "consume the driver
        // and own it from now on". A second call sees `None` and
        // returns an error.
        let driver = self
            .driver_slot
            .lock()
            .expect("driver slot mutex not poisoned")
            .take()
            .ok_or_else(|| ConnectorError::stack(AlreadyRegistered))?;

        // Spawn the dispatcher loop on the gateway's tokio runtime
        // (REQ_0321). Bring-up happens inside the task — CycleRunner::new
        // is async because the driver's `bring_up` is async.
        let handle = self
            .gateway
            .handle()
            .ok_or_else(|| ConnectorError::stack(GatewayShutDown))?;
        let registry = Arc::clone(self.state.registry());
        let health = self.state.health_arc();
        let options = self.state.options().clone();
        let stop = self.state.stop_signal();
        let cycle_period = options.cycle_time();
        handle.spawn(async move {
            let runner = match CycleRunner::new(driver, &options, health).await {
                Ok(r) => r,
                Err(e) => {
                    // Bring-up failure already drove the health
                    // monitor; nothing else to do but exit. The
                    // dispatcher task ending without panic is the
                    // standard signal — observers learn via the
                    // health subscription.
                    let _ = e;
                    return;
                }
            };
            // Loop until `stop` flips. Errors are logged via the
            // task's natural return path; a future commit can add
            // structured tracing once the framework's tracing crate
            // is wired in.
            let _ = dispatcher_loop(registry, runner, stop, cycle_period).await;
        });

        // Heartbeat ExecutableItem so the connector is a well-formed
        // ConnectorHost participant per REQ_0272. The dispatcher does
        // the real work; this item exists to satisfy the
        // executor-registration contract.
        let cycle = self.state.options().cycle_time();
        let heartbeat = item_with_triggers(
            move |d| {
                d.interval(cycle);
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
        if routing.direction != PdoDirection::Rx {
            return Err(ConnectorError::InvalidDescriptor(format!(
                "create_writer requires an RxPDO routing (MainDevice → SubDevice), got {:?}",
                routing.direction
            )));
        }
        let svc_name = service_name(descriptor.name(), &Direction::Outbound);
        let plugin_desc = ChannelDescriptor::<EthercatRouting, N>::new(svc_name.clone(), routing)?;
        let factory = self.factory();
        // Plugin-side publisher (returned to caller).
        let writer = factory.create_writer::<T, _, _, N>(&plugin_desc, self.codec.clone())?;
        // Gateway-side raw subscriber — drains plugin's publishes into
        // PDI on each cycle. Codec is bypassed (REQ_0327's amendment).
        let raw_reader = factory.create_raw_reader_named::<N>(&svc_name)?;
        let drain: Box<dyn crate::OutboundDrain> = Box::new(IoxOutboundDrain::new(raw_reader));
        self.state
            .registry()
            .lock()
            .expect("registry mutex not poisoned")
            .register(
                descriptor.name().to_string(),
                routing,
                PdoDirection::Rx,
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
        if routing.direction != PdoDirection::Tx {
            return Err(ConnectorError::InvalidDescriptor(format!(
                "create_reader requires a TxPDO routing (SubDevice → MainDevice), got {:?}",
                routing.direction
            )));
        }
        let svc_name = service_name(descriptor.name(), &Direction::Inbound);
        let plugin_desc = ChannelDescriptor::<EthercatRouting, N>::new(svc_name.clone(), routing)?;
        let factory = self.factory();
        // Plugin-side subscriber (returned to caller).
        let reader = factory.create_reader::<T, _, _, N>(&plugin_desc, self.codec.clone())?;
        // Gateway-side raw publisher — feeds PDI inputs back to the
        // plugin each cycle.
        let raw_writer = factory.create_raw_writer_named::<N>(&svc_name)?;
        let publish: Box<dyn crate::InboundPublish> = Box::new(IoxInboundPublish::new(raw_writer));
        self.state
            .registry()
            .lock()
            .expect("registry mutex not poisoned")
            .register(
                descriptor.name().to_string(),
                routing,
                PdoDirection::Tx,
                ChannelBinding::Inbound(publish),
            );
        Ok(reader)
    }
}

enum Direction {
    Outbound,
    Inbound,
}

fn service_name(base: &str, direction: &Direction) -> String {
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
            "ethercat connector: register_with already called; driver was moved into dispatcher"
        )
    }
}

impl std::error::Error for AlreadyRegistered {}

#[derive(Debug)]
struct GatewayShutDown;

impl core::fmt::Display for GatewayShutDown {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "ethercat connector: gateway runtime is shut down")
    }
}

impl std::error::Error for GatewayShutDown {}
