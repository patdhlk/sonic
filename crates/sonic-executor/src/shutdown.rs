//! Optional Ctrl-C → [`Stoppable::stop`] bridge.

use crate::context::Stoppable;

/// Install a process-wide Ctrl-C handler that calls `stop.stop()` on SIGINT.
/// No-op if the `ctrlc-default` feature is disabled. Idempotent: only the
/// first call installs the handler; later calls are silently ignored.
#[cfg(feature = "ctrlc-default")]
pub(crate) fn install_ctrlc(stop: Stoppable) -> Result<(), crate::error::ExecutorError> {
    static INSTALLED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    INSTALLED.get_or_init(|| {
        let _ = ctrlc::set_handler(move || {
            stop.stop();
        });
    });
    Ok(())
}

#[cfg(not(feature = "ctrlc-default"))]
pub(crate) fn install_ctrlc(_: Stoppable) -> Result<(), crate::error::ExecutorError> {
    Ok(())
}
