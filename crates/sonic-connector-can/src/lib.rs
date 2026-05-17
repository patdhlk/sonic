//! CAN (SocketCAN) reference connector — `BB_0070` / `FEAT_0046`.
//!
//! Layer-1 (always available — portable across Linux / macOS /
//! Windows for development and testing):
//!
//! * [`routing`] — typed `CanRouting`, `CanIface`, `CanId`,
//!   `CanFrameKind`, `CanFdFlags` (`REQ_0601`, `REQ_0615`).
//! * [`options::CanConnectorOptions`] typed builder (`REQ_0506`,
//!   `REQ_0520`, `REQ_0534`).
//! * [`bridge::OutboundBridge`] / [`bridge::InboundBridge`]
//!   (`REQ_0506`–`REQ_0608`).
//! * [`health::CanHealthMonitor`] with per-interface worst-of
//!   aggregation (`REQ_0630`, `REQ_0635`).
//! * [`registry::ChannelRegistry`] — per-iface routing registry
//!   (`REQ_0525`).
//! * [`filter::PerIfaceFilter`] — union compiler + match predicate
//!   (`BB_0074`, `REQ_0622`, `REQ_0623`, `REQ_0624`).
//! * [`driver::CanInterfaceLike`] — async trait every back-end
//!   implements (`BB_0072`).
//! * [`mock::MockCanInterface`] — in-process loopback for layer-1
//!   tests (`BB_0075`, `REQ_0504`).
//! * [`gateway::CanGateway`] — owns the per-gateway tokio runtime
//!   (`REQ_0505`).
//! * [`dispatcher`] — per-iface RX/TX loops + error classifier +
//!   bus-off reconnect (`ARCH_0061`, `ARCH_0062`).
//! * [`connector::CanConnector`] — implements
//!   [`sonic_connector_host::Connector`] (`REQ_0600`).
//!
//! Layer-2 (Linux-only, gated behind the default-off
//! `socketcan-integration` cargo feature per `REQ_0503` / `REQ_0502`):
//!
//! * `RealCanInterface` (in `real` module — compiled only when both
//!   `feature = "socketcan-integration"` and
//!   `target_os = "linux"` hold) — wraps
//!   `socketcan::tokio::CanFdSocket`. Always FD-aware (one socket
//!   per interface handles both classical and FD frames); error
//!   frames enabled at open (`REQ_0631`). Verified by the
//!   `tests/vcan_smoke.rs` integration test (`TEST_0512`) when the
//!   kernel `vcan` module is loaded.
//!
//! [`HealthEvent`]: sonic_connector_core::HealthEvent

#![warn(missing_docs)]
// Allow CAN domain identifiers (CAN_RAW_FILTER, CAP_NET_RAW, BRS,
// ESI, etc.) to appear in docstrings without backticks. Matches the
// posture taken by the EtherCAT crate for analogous fieldbus
// terminology.
#![allow(clippy::doc_markdown)]

pub mod bridge;
pub mod connector;
pub mod dispatcher;
pub mod driver;
pub mod filter;
pub mod gateway;
pub mod health;
pub mod mock;
pub mod options;
#[cfg(all(feature = "socketcan-integration", target_os = "linux"))]
pub mod real;
pub mod registry;
pub mod routing;

pub use bridge::{InboundBridge, InboundOutcome, OutboundBridge, OutboundError};
pub use connector::CanConnector;
pub use dispatcher::{
    DispatcherCommand, IoxInboundPublish, IoxOutboundDrain, IterationOutcome,
    dispatch_one_iteration, dispatcher_loop,
};
pub use driver::{
    CanData, CanErrorKind, CanFilter, CanFrame, CanIfaceState, CanInterfaceLike, CanIoError,
};
pub use filter::{PerIfaceFilter, matches};
pub use gateway::CanGateway;
pub use health::{CanHealthMonitor, IfaceHealthKind};
pub use mock::{MockCanInterface, MockCanState};
pub use options::{CanConnectorOptions, CanConnectorOptionsBuilder};
#[cfg(all(feature = "socketcan-integration", target_os = "linux"))]
pub use real::RealCanInterface;
pub use registry::{
    ChannelBinding, ChannelHandle, ChannelRegistry, Direction, InboundPublish, OutboundDrain,
    RegisteredChannel,
};
pub use routing::{CanFdFlags, CanFrameKind, CanId, CanIface, CanRouting};
