//! [`CanGateway`] — owns the per-gateway tokio runtime that hosts the
//! per-iface RX / TX tasks. `REQ_0505`, `ADR_0026`.

use std::time::Duration;

use tokio::runtime::{Builder as RtBuilder, Handle, Runtime};

use crate::options::CanConnectorOptions;

/// Default shutdown budget when joining the gateway's tokio runtime
/// on `Drop`. Matches the EtherCAT crate's `DEFAULT_SHUTDOWN_BUDGET`.
pub const DEFAULT_SHUTDOWN_BUDGET: Duration = Duration::from_secs(5);

/// Gateway-side container. Owns the tokio runtime (`REQ_0505`) and
/// exposes a handle for spawning per-iface dispatcher tasks.
#[derive(Debug)]
pub struct CanGateway {
    options: CanConnectorOptions,
    runtime: Option<Runtime>,
    shutdown_budget: Duration,
}

impl CanGateway {
    /// Construct the gateway and start its tokio runtime.
    ///
    /// # Errors
    ///
    /// Returns the tokio runtime error verbatim if runtime
    /// construction fails (e.g. OS denies thread creation under
    /// resource pressure).
    pub fn new(options: CanConnectorOptions) -> std::io::Result<Self> {
        let runtime = build_runtime(options.tokio_worker_threads())?;
        Ok(Self {
            options,
            runtime: Some(runtime),
            shutdown_budget: DEFAULT_SHUTDOWN_BUDGET,
        })
    }

    /// Construct with a custom shutdown budget. Used by tests that
    /// want a tighter timeout.
    pub fn with_shutdown_budget(
        options: CanConnectorOptions,
        budget: Duration,
    ) -> std::io::Result<Self> {
        let mut gw = Self::new(options)?;
        gw.shutdown_budget = budget;
        Ok(gw)
    }

    /// Borrow the gateway's options.
    #[must_use]
    pub const fn options(&self) -> &CanConnectorOptions {
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

impl Drop for CanGateway {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(self.shutdown_budget);
        }
    }
}

fn build_runtime(worker_threads: usize) -> std::io::Result<Runtime> {
    let mut builder = RtBuilder::new_multi_thread();
    builder.worker_threads(worker_threads.max(1));
    builder.thread_name("sonic-can");
    builder.enable_time();
    builder.build()
}
