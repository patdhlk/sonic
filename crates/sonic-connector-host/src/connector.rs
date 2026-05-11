//! The framework's [`Connector`] trait. `REQ_0220`–`REQ_0224`.
//!
//! Implementations live in protocol crates (e.g.
//! `sonic-connector-mqtt`, `sonic-connector-ethercat`). The trait is
//! intentionally not dyn-compatible — it carries associated `Routing` /
//! `Codec` types plus generic-on-`T` methods. Heterogeneous
//! collections of connectors are handled at the host level via the
//! [`crate::ConnectorHost::register`] entry point, which takes one
//! concrete connector at a time and runs its registration logic
//! against the host's [`sonic_executor::Executor`].

use sonic_connector_core::{
    ChannelDescriptor, ConnectorError, ConnectorHealth, PayloadCodec, Routing,
};
use sonic_connector_transport_iox::{ChannelReader, ChannelWriter};
use sonic_executor::Executor;

use crate::HealthSubscription;

/// One concrete connector — the bridge between an application and a
/// specific external protocol (MQTT, `EtherCAT`, OPC UA, ...).
///
/// Implementations carry their own `Routing` type (`REQ_0224`) and pick
/// a `PayloadCodec` (`REQ_0211`). Both are associated types so the
/// connector's [`create_writer`](Connector::create_writer) /
/// [`create_reader`](Connector::create_reader) methods can return
/// concrete `ChannelWriter<T, Codec, N>` / `ChannelReader<T, Codec, N>`
/// handles (`REQ_0223`).
///
/// The trait is `Send + 'static` because connectors cross thread
/// boundaries between the [`sonic_executor::Executor`]'s `WaitSet`
/// thread and the connector's internal tokio sidecar (where
/// applicable). It is **not** `Sync` — concrete connectors typically
/// hold an `Arc` to their shared state internally and expose only
/// `&self` methods that derive cheap clones.
pub trait Connector: Send + 'static {
    /// The protocol-specific routing type carried by every
    /// [`ChannelDescriptor`] this connector accepts.
    type Routing: Routing;

    /// The codec the connector parameterises its channel handles on.
    type Codec: PayloadCodec;

    /// Human-readable connector name. Used in logs and tracing spans.
    fn name(&self) -> &str;

    /// Snapshot the connector's current health. Cheap to call (no
    /// blocking, no allocation).
    fn health(&self) -> ConnectorHealth;

    /// Receive-only handle over the connector's [`HealthEvent`] stream
    /// (`REQ_0231`). Multiple subscribers receive the same events; the
    /// subscription is unbounded.
    ///
    /// [`HealthEvent`]: sonic_connector_core::HealthEvent
    fn subscribe_health(&self) -> HealthSubscription;

    /// Register this connector's `ExecutableItem` instances with
    /// `executor` (`REQ_0272`). Called once by
    /// [`crate::ConnectorHost::register`].
    ///
    /// Concrete connectors typically construct a small executable item
    /// that drives their protocol-side work (e.g. tokio sidecar
    /// ticking, bridge draining) and submit it via
    /// [`Executor::add`](sonic_executor::Executor::add).
    fn register_with(&mut self, executor: &mut Executor) -> Result<(), ConnectorError>;

    /// Open a writer for `descriptor` (`REQ_0223`).
    fn create_writer<T, const N: usize>(
        &self,
        descriptor: &ChannelDescriptor<Self::Routing, N>,
    ) -> Result<ChannelWriter<T, Self::Codec, N>, ConnectorError>
    where
        T: serde::Serialize;

    /// Open a reader for `descriptor` (`REQ_0223`).
    fn create_reader<T, const N: usize>(
        &self,
        descriptor: &ChannelDescriptor<Self::Routing, N>,
    ) -> Result<ChannelReader<T, Self::Codec, N>, ConnectorError>
    where
        T: serde::de::DeserializeOwned;
}
