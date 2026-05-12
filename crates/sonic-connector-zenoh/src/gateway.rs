//! [`ZenohGateway`] — owns the per-gateway tokio runtime that hosts
//! the dispatcher loop and Zenoh session callbacks (`REQ_0403`).
//!
//! Mirrors `sonic_connector_ethercat::gateway::EthercatGateway` —
//! same shutdown-budget-on-Drop semantics, same `Handle` accessor.

use std::time::Duration;

use tokio::runtime::{Builder as RtBuilder, Handle, Runtime};

use crate::options::ZenohConnectorOptions;

/// Default shutdown budget when joining the gateway's tokio runtime
/// on `Drop`. Mirrors `EthercatGateway::DEFAULT_SHUTDOWN_BUDGET`.
pub const DEFAULT_SHUTDOWN_BUDGET: Duration = Duration::from_secs(5);

/// Gateway-side container. Owns the tokio runtime (`REQ_0403`); the
/// dispatcher loop spawned on this runtime drives all Zenoh I/O.
#[derive(Debug)]
pub struct ZenohGateway {
    options: ZenohConnectorOptions,
    runtime: Option<Runtime>,
    shutdown_budget: Duration,
}

impl ZenohGateway {
    /// Construct the gateway and start its tokio runtime.
    ///
    /// # Errors
    ///
    /// Returns the tokio runtime error verbatim if runtime
    /// construction fails (e.g. the OS denies thread creation under
    /// resource pressure).
    pub fn new(options: ZenohConnectorOptions) -> std::io::Result<Self> {
        let runtime = build_runtime(options.tokio_worker_threads)?;
        Ok(Self {
            options,
            runtime: Some(runtime),
            shutdown_budget: DEFAULT_SHUTDOWN_BUDGET,
        })
    }

    /// Construct the gateway with a custom shutdown budget. Useful in
    /// tests that want a tighter timeout.
    ///
    /// # Errors
    ///
    /// Same as [`Self::new`].
    pub fn with_shutdown_budget(
        options: ZenohConnectorOptions,
        budget: Duration,
    ) -> std::io::Result<Self> {
        let mut gw = Self::new(options)?;
        gw.shutdown_budget = budget;
        Ok(gw)
    }

    /// Borrow the gateway's options.
    #[must_use]
    pub const fn options(&self) -> &ZenohConnectorOptions {
        &self.options
    }

    /// Borrow a tokio [`Handle`] for spawning work on the gateway's
    /// runtime. Returns `None` after `Drop` has consumed the runtime.
    #[must_use]
    pub fn handle(&self) -> Option<Handle> {
        self.runtime.as_ref().map(Runtime::handle).cloned()
    }

    /// Shutdown budget honoured by the `Drop` impl.
    #[must_use]
    pub const fn shutdown_budget(&self) -> Duration {
        self.shutdown_budget
    }
}

impl Drop for ZenohGateway {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(self.shutdown_budget);
        }
    }
}

fn build_runtime(worker_threads: usize) -> std::io::Result<Runtime> {
    let mut builder = RtBuilder::new_multi_thread();
    builder.worker_threads(worker_threads.max(1));
    builder.thread_name("sonic-zenoh");
    builder.enable_all();
    builder.build()
}
