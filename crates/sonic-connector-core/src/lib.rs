//! sonic-connector-core — framework-level traits and types shared by every
//! sonic-connector protocol crate.
//!
//! Implements [`BB_0001`](../../spec/architecture/connector.rst). Realises:
//!
//! * [`Routing`] — marker trait (`REQ_0222`).
//! * [`ChannelDescriptor`] — typed channel description (`REQ_0221`).
//! * [`PayloadCodec`] — compile-time codec dispatch (`REQ_0210`, `REQ_0211`).
//! * [`ConnectorHealth`] / [`HealthEvent`] — uniform observable health
//!   (`REQ_0230`, `REQ_0234`) with the `ARCH_0012` state machine.
//! * [`ReconnectPolicy`] / [`ExponentialBackoff`] — backoff for stacks that
//!   surface raw connect events (`REQ_0232`, `REQ_0233`).
//! * [`ConnectorError`] — framework error type (`REQ_0213`, `REQ_0214`,
//!   `REQ_0323`).
//!
//! The `Connector` trait itself and the concrete `ChannelWriter` /
//! `ChannelReader` handles live in `sonic-connector-transport-iox`
//! (`BB_0002`) — they bind iceoryx2 directly and would create a cyclic
//! dependency if included here.

#![warn(missing_docs)]
#![deny(unsafe_code)]

pub mod codec;
pub mod descriptor;
pub mod error;
pub mod health;
pub mod reconnect;
pub mod routing;

pub use codec::PayloadCodec;
pub use descriptor::ChannelDescriptor;
pub use error::ConnectorError;
pub use health::{
    ConnectorHealth, ConnectorHealthKind, HealthEvent, HealthMonitor, IllegalTransition,
};
pub use reconnect::{ExponentialBackoff, ExponentialBackoffBuilder, ReconnectPolicy};
pub use routing::Routing;
