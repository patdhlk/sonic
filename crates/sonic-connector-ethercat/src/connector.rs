//! [`EthercatConnector`] — plugin-side implementation of the
//! framework's [`Connector`] trait. `REQ_0310`.
//!
//! Holds an iceoryx2 [`Node`] for opening pub/sub services, the
//! application's codec, and a shared [`EthercatHealthMonitor`] (the
//! gateway broadcasts health transitions through the same monitor;
//! the plugin observes them via `subscribe_health`).
//!
//! Until `ethercrab` integration lands, `register_with` registers a
//! no-op interval-triggered item so the connector is a well-formed
//! `Connector` participant in [`sonic_connector_host::ConnectorHost`].

use std::sync::Arc;

use iceoryx2::node::Node;
use iceoryx2::prelude::{NodeBuilder, ipc};
use sonic_connector_core::{ChannelDescriptor, ConnectorError, ConnectorHealth};
use sonic_connector_host::{Connector, HealthSubscription};
use sonic_connector_transport_iox::{ChannelReader, ChannelWriter, ServiceFactory};
use sonic_executor::{ControlFlow, Executor, item_with_triggers};

use crate::health::EthercatHealthMonitor;
use crate::options::EthercatConnectorOptions;
use crate::routing::EthercatRouting;

/// Plugin-side `EtherCAT` connector. Generic over a `PayloadCodec`
/// (`REQ_0211`).
pub struct EthercatConnector<C> {
    /// Shared state — the gateway side holds a clone of the
    /// [`EthercatHealthMonitor`] so its transitions surface on the
    /// plugin's subscription.
    state: Arc<EthercatState>,
    codec: C,
    /// iceoryx2 node owned by the plugin side. The gateway side has
    /// its own node; the two sides communicate through service-name
    /// rendezvous, not through a shared node.
    node: Arc<Node<ipc::Service>>,
}

/// Connector-internal state shared between [`EthercatConnector`] and
/// [`crate::EthercatGateway`] in deployments where both halves live in
/// the same process.
#[derive(Debug)]
pub struct EthercatState {
    health: EthercatHealthMonitor,
    options: EthercatConnectorOptions,
}

impl EthercatState {
    /// Construct connector-internal state from configured options.
    #[must_use]
    pub fn new(options: EthercatConnectorOptions) -> Self {
        Self {
            health: EthercatHealthMonitor::new(),
            options,
        }
    }

    /// Borrow the shared health monitor.
    #[must_use]
    pub const fn health(&self) -> &EthercatHealthMonitor {
        &self.health
    }

    /// Borrow the configured options.
    #[must_use]
    pub const fn options(&self) -> &EthercatConnectorOptions {
        &self.options
    }
}

impl<C> EthercatConnector<C> {
    /// Construct a plugin-side connector. Opens a fresh iceoryx2 node
    /// for this connector instance.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectorError::Stack`] wrapping any iceoryx2 node
    /// creation failure.
    pub fn new(state: Arc<EthercatState>, codec: C) -> Result<Self, ConnectorError> {
        let node = NodeBuilder::new()
            .create::<ipc::Service>()
            .map_err(|e| ConnectorError::stack(NodeError(format!("{e:?}"))))?;
        Ok(Self {
            state,
            codec,
            node: Arc::new(node),
        })
    }

    /// Borrow the shared state. Useful for testing and for
    /// constructing gateway-side helpers that share the health
    /// monitor with the plugin.
    #[must_use]
    pub const fn state(&self) -> &Arc<EthercatState> {
        &self.state
    }
}

impl<C> Connector for EthercatConnector<C>
where
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
        // Placeholder item — drained on every WaitSet wakeup. Until
        // `ethercrab` integration lands, the gateway side does not
        // publish work for this item to do. Registering a no-op item
        // keeps the connector a well-formed `ConnectorHost::register`
        // participant.
        let cycle = self.state.options().cycle_time();
        let item = item_with_triggers(
            move |d| {
                d.interval(cycle);
                Ok(())
            },
            |_ctx| Ok(ControlFlow::Continue),
        );
        executor.add(item).map_err(ConnectorError::stack)?;
        Ok(())
    }

    fn create_writer<T, const N: usize>(
        &self,
        descriptor: &ChannelDescriptor<Self::Routing, N>,
    ) -> Result<ChannelWriter<T, Self::Codec, N>, ConnectorError>
    where
        T: serde::Serialize,
    {
        let factory = ServiceFactory::new(&self.node);
        factory.create_writer::<T, _, _, N>(descriptor, self.codec.clone())
    }

    fn create_reader<T, const N: usize>(
        &self,
        descriptor: &ChannelDescriptor<Self::Routing, N>,
    ) -> Result<ChannelReader<T, Self::Codec, N>, ConnectorError>
    where
        T: serde::de::DeserializeOwned,
    {
        let factory = ServiceFactory::new(&self.node);
        factory.create_reader::<T, _, _, N>(descriptor, self.codec.clone())
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
