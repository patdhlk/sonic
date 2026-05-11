//! [`ConnectorGateway`] — gateway-side composition. Parallel to
//! [`crate::ConnectorHost`] but used by the gateway binary (or in-
//! process tokio task) that owns the protocol stack itself
//! (`REQ_0271`).
//!
//! The gateway side runs its own [`sonic_executor::Executor`] — the
//! plugin and gateway never share an executor. Cross-process
//! communication flows over iceoryx2 (`REQ_0240`), and in-process
//! deployments still keep two distinct `Executor` instances so the
//! plugin's `WaitSet` is not perturbed by gateway-side bookkeeping.
//!
//! Apart from the deployment-side semantics, the gateway's builder
//! mirrors the host's. The duplication is intentional: keeping the
//! types nominally distinct lets call-site code make its deployment
//! intent visible at a glance.

use sonic_connector_core::ConnectorError;
use sonic_executor::{Executor, Stoppable};

use crate::Connector;

/// Gateway-side composition root.
pub struct ConnectorGateway {
    executor: Executor,
    connector_names: Vec<String>,
}

impl ConnectorGateway {
    /// Start a builder.
    #[must_use]
    pub fn builder() -> ConnectorGatewayBuilder {
        ConnectorGatewayBuilder::default()
    }

    /// Register a gateway-side connector. Same semantics as
    /// [`crate::ConnectorHost::register`].
    pub fn register<C>(&mut self, mut connector: C) -> Result<C, ConnectorError>
    where
        C: Connector,
    {
        let name = connector.name().to_owned();
        connector.register_with(&mut self.executor)?;
        self.connector_names.push(name);
        Ok(connector)
    }

    /// Borrow the underlying [`Executor`]. Escape hatch.
    pub const fn executor_mut(&mut self) -> &mut Executor {
        &mut self.executor
    }

    /// A [`Stoppable`] for ending the gateway's run loop.
    #[must_use]
    pub fn stoppable(&self) -> Stoppable {
        self.executor.stoppable()
    }

    /// Run the gateway's executor until stop or error.
    pub fn run(&mut self) -> Result<(), ConnectorError> {
        self.executor.run().map_err(ConnectorError::stack)
    }

    /// Run for at most `max` wall-clock duration.
    pub fn run_for(&mut self, max: std::time::Duration) -> Result<(), ConnectorError> {
        self.executor.run_for(max).map_err(ConnectorError::stack)
    }

    /// Run for `n` barrier-cycles.
    pub fn run_n(&mut self, n: usize) -> Result<(), ConnectorError> {
        self.executor.run_n(n).map_err(ConnectorError::stack)
    }

    /// Names of registered connectors, in registration order.
    #[must_use]
    pub fn connector_names(&self) -> &[String] {
        &self.connector_names
    }
}

/// Builder for [`ConnectorGateway`].
#[derive(Default)]
pub struct ConnectorGatewayBuilder {
    worker_threads: Option<usize>,
}

impl ConnectorGatewayBuilder {
    /// Number of worker threads for the gateway's pool.
    #[must_use]
    pub const fn worker_threads(mut self, n: usize) -> Self {
        self.worker_threads = Some(n);
        self
    }

    /// Finalise the gateway.
    pub fn build(self) -> Result<ConnectorGateway, ConnectorError> {
        let mut builder = Executor::builder();
        if let Some(n) = self.worker_threads {
            builder = builder.worker_threads(n);
        }
        let executor = builder.build().map_err(ConnectorError::stack)?;
        Ok(ConnectorGateway {
            executor,
            connector_names: Vec::new(),
        })
    }
}
