//! [`ZenohConnector`] — plugin-side implementation of the framework's
//! `Connector` trait (`REQ_0400`).
//!
//! Generic over a [`ZenohSessionLike`] back-end (the session — mock
//! in tests, real in Z4) and a `PayloadCodec` (`REQ_0211`).
//!
//! Z2 Task 5 lands the struct + constructor only; the `Connector`
//! trait impl is added by Z2 Task 6.

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
    /// connector's node. Used by Z2 Task 6's `create_writer` /
    /// `create_reader`.
    #[allow(dead_code)]
    pub(crate) fn factory(&self) -> ServiceFactory<'_> {
        ServiceFactory::new(&self.node)
    }

    /// Internal: borrow the gateway (used by Z2 Task 6's
    /// `register_with` to obtain a tokio `Handle`).
    #[allow(dead_code)]
    pub(crate) const fn gateway(&self) -> &ZenohGateway {
        &self.gateway
    }

    /// Internal: clone the codec (used by Z2 Task 6's `create_writer` /
    /// `create_reader`).
    #[allow(dead_code)]
    pub(crate) const fn codec(&self) -> &C {
        &self.codec
    }

    /// Internal: take the session out of its slot (called once, by Z2
    /// Task 6's `register_with`).
    #[allow(dead_code)]
    pub(crate) fn take_session(&self) -> Option<Arc<S>> {
        self.session_slot
            .lock()
            .expect("session slot mutex not poisoned")
            .take()
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
