//! Optional Ctrl-C → [`Stoppable::stop`] bridge.

// pub(crate) inside a private module — intentional, executor.rs uses it.
#![allow(clippy::redundant_pub_crate)]

use crate::context::Stoppable;

/// Install a process-wide Ctrl-C handler that calls `stop.stop()` on SIGINT.
/// No-op if the `ctrlc` feature is disabled. Idempotent: only the
/// first call installs the handler; later calls are silently ignored.
#[cfg(feature = "ctrlc")]
#[allow(clippy::unnecessary_wraps)]
pub(crate) fn install_ctrlc(stop: Stoppable) -> Result<(), crate::error::ExecutorError> {
    static INSTALLED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    INSTALLED.get_or_init(|| {
        let _ = ctrlc::set_handler(move || {
            stop.stop();
        });
    });
    Ok(())
}

/// No-op fallback when `ctrlc` feature is disabled.
#[cfg(not(feature = "ctrlc"))]
#[allow(clippy::unnecessary_wraps)]
pub(crate) fn install_ctrlc(_: Stoppable) -> Result<(), crate::error::ExecutorError> {
    Ok(())
}
