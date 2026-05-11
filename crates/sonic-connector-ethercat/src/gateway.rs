//! [`EthercatGateway`] — owns the per-gateway tokio runtime that hosts
//! the ethercrab TX/RX task once `ethercrab` integration lands.
//! `ADR_0026`, `REQ_0321`.
//!
//! In this commit (C5a) the gateway does not yet drive a real
//! `ethercrab::MainDevice`. Its surface is intentionally minimal: it
//! constructs a tokio multi-thread runtime sized per
//! [`EthercatConnectorOptions::tokio_worker_threads`], exposes a
//! handle for spawning work, and shuts the runtime down on `Drop`
//! with a configurable budget (default 5 seconds, mirroring
//! `ARCH_0013`'s shutdown coordination).

use std::time::Duration;

use tokio::runtime::{Builder as RtBuilder, Handle, Runtime};

use crate::options::EthercatConnectorOptions;

/// Default shutdown budget when joining the gateway's tokio runtime
/// on `Drop` (`ADR_0026`).
pub const DEFAULT_SHUTDOWN_BUDGET: Duration = Duration::from_secs(5);

/// Gateway-side container. Owns the tokio runtime (`REQ_0321`,
/// `ADR_0026`); future commits add the ethercrab `MainDevice` and the
/// cycle loop that drives it.
#[derive(Debug)]
pub struct EthercatGateway {
    options: EthercatConnectorOptions,
    runtime: Option<Runtime>,
    shutdown_budget: Duration,
}

impl EthercatGateway {
    /// Construct the gateway and start its tokio runtime.
    ///
    /// # Errors
    ///
    /// Returns the tokio runtime error verbatim if runtime
    /// construction fails (e.g. the OS denies thread creation under
    /// resource pressure).
    pub fn new(options: EthercatConnectorOptions) -> std::io::Result<Self> {
        let runtime = build_runtime(options.tokio_worker_threads())?;
        Ok(Self {
            options,
            runtime: Some(runtime),
            shutdown_budget: DEFAULT_SHUTDOWN_BUDGET,
        })
    }

    /// Construct the gateway with a custom shutdown budget. Useful in
    /// tests that want a tighter timeout.
    pub fn with_shutdown_budget(
        options: EthercatConnectorOptions,
        budget: Duration,
    ) -> std::io::Result<Self> {
        let mut gw = Self::new(options)?;
        gw.shutdown_budget = budget;
        Ok(gw)
    }

    /// Borrow the gateway's options.
    #[must_use]
    pub const fn options(&self) -> &EthercatConnectorOptions {
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

impl Drop for EthercatGateway {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(self.shutdown_budget);
        }
    }
}

fn build_runtime(worker_threads: usize) -> std::io::Result<Runtime> {
    let mut builder = RtBuilder::new_multi_thread();
    builder.worker_threads(worker_threads.max(1));
    builder.thread_name("sonic-ethercat");
    builder.build()
}
