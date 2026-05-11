//! [`ConnectorHost`] — plugin-side composition. Owns a
//! [`sonic_executor::Executor`] and registers connector-contributed
//! `ExecutableItem`s with it (`REQ_0270`, `REQ_0272`).
//!
//! Typical usage:
//!
//! ```ignore
//! let host = ConnectorHost::builder().worker_threads(2).build()?;
//! let mut host = host;
//! let mq = host.register(MqttConnector::new(mqtt_opts)?)?;
//! let writer = mq.create_writer(&channel_descriptor)?;
//! host.run()?;
//! ```

use sonic_connector_core::ConnectorError;
use sonic_executor::{Executor, Stoppable};

use crate::Connector;

/// Plugin-side composition root. Owns the
/// [`sonic_executor::Executor`] and connector-contributed
/// `ExecutableItem`s.
pub struct ConnectorHost {
    executor: Executor,
    connector_names: Vec<String>,
}

impl ConnectorHost {
    /// Start a builder.
    #[must_use]
    pub fn builder() -> ConnectorHostBuilder {
        ConnectorHostBuilder::default()
    }

    /// Register `connector` with the host (`REQ_0272`).
    ///
    /// Calls [`Connector::register_with`] with the host's executor,
    /// then returns the connector by value so the caller can keep it
    /// for further API calls (`create_writer`, `create_reader`,
    /// `subscribe_health`).
    ///
    /// # Errors
    ///
    /// Returns any error surfaced by the connector's registration —
    /// typically `ConnectorError::Stack` wrapping the executor's
    /// error.
    pub fn register<C>(&mut self, mut connector: C) -> Result<C, ConnectorError>
    where
        C: Connector,
    {
        let name = connector.name().to_owned();
        connector.register_with(&mut self.executor)?;
        self.connector_names.push(name);
        Ok(connector)
    }

    /// Borrow the underlying [`Executor`]. Escape hatch for power
    /// users that need to add non-connector `ExecutableItem`s.
    pub const fn executor_mut(&mut self) -> &mut Executor {
        &mut self.executor
    }

    /// A [`Stoppable`] handle for ending the host's run loop from
    /// another thread (e.g. a signal handler). The handle is waker-
    /// aware from the moment of construction.
    #[must_use]
    pub fn stoppable(&self) -> Stoppable {
        self.executor.stoppable()
    }

    /// Run the host's executor until stop or error
    /// (`Executor::run`'s contract).
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

    /// Snapshot of the names of registered connectors (in registration
    /// order). Useful for diagnostics.
    #[must_use]
    pub fn connector_names(&self) -> &[String] {
        &self.connector_names
    }
}

/// Builder for [`ConnectorHost`].
#[derive(Default)]
pub struct ConnectorHostBuilder {
    worker_threads: Option<usize>,
}

impl ConnectorHostBuilder {
    /// Number of worker threads for the host's pool. `0` selects
    /// inline mode (no pool). `None` (default) lets the executor pick
    /// `num_cpus::get_physical()`.
    #[must_use]
    pub const fn worker_threads(mut self, n: usize) -> Self {
        self.worker_threads = Some(n);
        self
    }

    /// Finalise the host. Builds the underlying
    /// [`sonic_executor::Executor`] with the configured worker-thread
    /// count.
    pub fn build(self) -> Result<ConnectorHost, ConnectorError> {
        let mut builder = Executor::builder();
        if let Some(n) = self.worker_threads {
            builder = builder.worker_threads(n);
        }
        let executor = builder.build().map_err(ConnectorError::stack)?;
        Ok(ConnectorHost {
            executor,
            connector_names: Vec::new(),
        })
    }
}
