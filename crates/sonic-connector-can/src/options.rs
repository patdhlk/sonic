//! [`CanConnectorOptions`] — typed builder configuring a
//! `CanConnector` / `CanGateway` pair. `REQ_0506`, `REQ_0520`,
//! `REQ_0534`.

use std::sync::Arc;
use std::time::Duration;

use sonic_connector_core::{ExponentialBackoff, ReconnectPolicy};

use crate::routing::CanIface;

/// Factory closure producing a fresh [`ReconnectPolicy`] instance.
/// Stored shared (`Arc`) so the connector can construct one policy
/// per iface on `register_with`. `Send + Sync` so the connector
/// itself crosses thread boundaries cleanly.
pub type ReconnectPolicyFactory = Arc<dyn Fn() -> Box<dyn ReconnectPolicy> + Send + Sync + 'static>;

/// Built `CanConnectorOptions`. Constructed via
/// [`CanConnectorOptionsBuilder`]; never mutated after build.
#[derive(Clone)]
pub struct CanConnectorOptions {
    ifaces: Vec<CanIface>,
    outbound_capacity: usize,
    inbound_capacity: usize,
    recovery_window: Duration,
    reconnect_policy_factory: ReconnectPolicyFactory,
    tokio_worker_threads: usize,
}

impl core::fmt::Debug for CanConnectorOptions {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CanConnectorOptions")
            .field("ifaces", &self.ifaces)
            .field("outbound_capacity", &self.outbound_capacity)
            .field("inbound_capacity", &self.inbound_capacity)
            .field("recovery_window", &self.recovery_window)
            .field(
                "reconnect_policy_factory",
                &"<Fn() -> Box<dyn ReconnectPolicy>>",
            )
            .field("tokio_worker_threads", &self.tokio_worker_threads)
            .finish()
    }
}

impl CanConnectorOptions {
    /// Start a builder with default values.
    #[must_use]
    pub fn builder() -> CanConnectorOptionsBuilder {
        CanConnectorOptionsBuilder::new()
    }

    /// Borrow the configured interface list (`REQ_0520`).
    #[must_use]
    pub fn ifaces(&self) -> &[CanIface] {
        &self.ifaces
    }

    /// Outbound bridge capacity per iface (`REQ_0506`). Default 256.
    #[must_use]
    pub const fn outbound_capacity(&self) -> usize {
        self.outbound_capacity
    }

    /// Inbound bridge capacity per iface (`REQ_0506`). Default 256.
    #[must_use]
    pub const fn inbound_capacity(&self) -> usize {
        self.inbound_capacity
    }

    /// Recovery debounce — time without further error frames before
    /// a `Degraded` iface returns to `Up` (`ARCH_0062`). Default 1 s.
    #[must_use]
    pub const fn recovery_window(&self) -> Duration {
        self.recovery_window
    }

    /// Construct a fresh reconnect policy instance used on bus-off
    /// (`REQ_0534`). Default is [`ExponentialBackoff`] with framework
    /// defaults. Each iface gets its own instance so backoff state
    /// does not bleed between ifaces.
    #[must_use]
    pub fn new_reconnect_policy(&self) -> Box<dyn ReconnectPolicy> {
        (self.reconnect_policy_factory)()
    }

    /// Borrow the shared factory closure.
    #[must_use]
    pub fn reconnect_policy_factory(&self) -> ReconnectPolicyFactory {
        Arc::clone(&self.reconnect_policy_factory)
    }

    /// Tokio worker-thread count for the gateway sidecar
    /// (`REQ_0505`). Default 1.
    #[must_use]
    pub const fn tokio_worker_threads(&self) -> usize {
        self.tokio_worker_threads
    }
}

/// Builder for [`CanConnectorOptions`].
pub struct CanConnectorOptionsBuilder {
    ifaces: Vec<CanIface>,
    outbound_capacity: usize,
    inbound_capacity: usize,
    recovery_window: Duration,
    reconnect_policy_factory: Option<ReconnectPolicyFactory>,
    tokio_worker_threads: usize,
}

impl CanConnectorOptionsBuilder {
    /// Construct a builder with default values:
    ///
    /// * `ifaces` — empty; must be set to at least one entry for a
    ///   useful gateway.
    /// * `outbound_capacity` / `inbound_capacity` — 256.
    /// * `recovery_window` — 1 s.
    /// * `reconnect_policy` — [`ExponentialBackoff::default`].
    /// * `tokio_worker_threads` — 1.
    #[must_use]
    pub fn new() -> Self {
        Self {
            ifaces: Vec::new(),
            outbound_capacity: 256,
            inbound_capacity: 256,
            recovery_window: Duration::from_secs(1),
            reconnect_policy_factory: None,
            tokio_worker_threads: 1,
        }
    }

    /// Append an interface to the gateway-owned set.
    #[must_use]
    pub fn iface(mut self, iface: CanIface) -> Self {
        self.ifaces.push(iface);
        self
    }

    /// Replace the iface list wholesale.
    #[must_use]
    pub fn ifaces(mut self, ifaces: impl IntoIterator<Item = CanIface>) -> Self {
        self.ifaces = ifaces.into_iter().collect();
        self
    }

    /// Override outbound bridge capacity (`REQ_0506`). Values below
    /// 1 are clamped to 1 at build time.
    #[must_use]
    pub const fn outbound_capacity(mut self, n: usize) -> Self {
        self.outbound_capacity = n;
        self
    }

    /// Override inbound bridge capacity (`REQ_0506`).
    #[must_use]
    pub const fn inbound_capacity(mut self, n: usize) -> Self {
        self.inbound_capacity = n;
        self
    }

    /// Override the per-iface recovery debounce.
    #[must_use]
    pub const fn recovery_window(mut self, d: Duration) -> Self {
        self.recovery_window = d;
        self
    }

    /// Override the reconnect-policy factory (`REQ_0534`). Each
    /// iface's dispatcher calls the factory once at construction so
    /// per-iface backoff state is independent.
    #[must_use]
    pub fn reconnect_policy_factory(mut self, f: ReconnectPolicyFactory) -> Self {
        self.reconnect_policy_factory = Some(f);
        self
    }

    /// Override the tokio worker-thread count (`REQ_0505`). Values
    /// below 1 are clamped to 1 at build time.
    #[must_use]
    pub const fn tokio_worker_threads(mut self, n: usize) -> Self {
        self.tokio_worker_threads = n;
        self
    }

    /// Finalise. Applies the capacity clamps (at least 1) and fills
    /// in the default reconnect-policy factory if none was set.
    #[must_use]
    pub fn build(self) -> CanConnectorOptions {
        let reconnect_policy_factory: ReconnectPolicyFactory = self
            .reconnect_policy_factory
            .unwrap_or_else(|| Arc::new(|| Box::new(ExponentialBackoff::default())));
        CanConnectorOptions {
            ifaces: self.ifaces,
            outbound_capacity: self.outbound_capacity.max(1),
            inbound_capacity: self.inbound_capacity.max(1),
            recovery_window: self.recovery_window,
            reconnect_policy_factory,
            tokio_worker_threads: self.tokio_worker_threads.max(1),
        }
    }
}

impl Default for CanConnectorOptionsBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_clamps() {
        let opts = CanConnectorOptions::builder()
            .outbound_capacity(0)
            .inbound_capacity(0)
            .tokio_worker_threads(0)
            .build();
        assert_eq!(opts.outbound_capacity(), 1);
        assert_eq!(opts.inbound_capacity(), 1);
        assert_eq!(opts.tokio_worker_threads(), 1);
        assert_eq!(opts.recovery_window(), Duration::from_secs(1));
        assert!(opts.ifaces().is_empty());
    }

    #[test]
    fn ifaces_list() {
        let a = CanIface::new("vcan0").unwrap();
        let b = CanIface::new("vcan1").unwrap();
        let opts = CanConnectorOptions::builder().iface(a).iface(b).build();
        assert_eq!(opts.ifaces().len(), 2);
        assert_eq!(opts.ifaces()[0].as_str(), "vcan0");
    }
}
