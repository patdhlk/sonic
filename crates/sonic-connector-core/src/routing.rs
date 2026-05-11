//! [`Routing`] — marker trait every connector's typed routing struct
//! implements. `REQ_0222`.

use core::fmt::Debug;

/// Marker trait every protocol-specific routing struct must implement.
///
/// `Routing` carries no methods of its own (`REQ_0222`); it only collects
/// the bounds the framework requires from every connector's routing type:
///
/// * `Clone` — the framework copies routing into [`ChannelDescriptor`] and
///   into pre-built dispatch closures.
/// * `Send + Sync + 'static` — routing values cross thread boundaries
///   between the `WaitSet` thread and the connector's tokio sidecar.
/// * `Debug` — routing appears in error messages and tracing spans.
///
/// Connectors define their own routing struct and `impl Routing for …`:
///
/// ```ignore
/// #[derive(Clone, Debug)]
/// pub struct MqttRouting {
///     pub topic: String,
///     pub qos: u8,
///     pub retained: bool,
/// }
///
/// impl Routing for MqttRouting {}
/// ```
///
/// [`ChannelDescriptor`]: crate::ChannelDescriptor
pub trait Routing: Clone + Send + Sync + Debug + 'static {}
