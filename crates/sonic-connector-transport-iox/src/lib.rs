//! iceoryx2-backed envelope transport for the sonic-connector framework.
//!
//! Implements [`BB_0002`](../../spec/architecture/connector.rst):
//!
//! * [`ConnectorEnvelope`] — POD wire format (`REQ_0200`, `REQ_0202`,
//!   `REQ_0203`, `REQ_0204`).
//! * [`ChannelWriter`] — zero-copy publisher using
//!   [`iceoryx2::port::publisher::Publisher::loan_uninit`] so the codec
//!   writes its bytes directly into shared memory (`REQ_0205`).
//! * [`ChannelReader`] — subscriber that decodes the envelope payload
//!   into `T` and surfaces codec errors as
//!   [`sonic_connector_core::ConnectorError::Codec`] (`REQ_0214`).
//! * [`ServiceFactory`] — opens / creates the iceoryx2 pub/sub service
//!   for a given [`sonic_connector_core::ChannelDescriptor`]
//!   (`REQ_0206`).
//!
//! The `Connector` trait itself (`REQ_0220`) lives in the host crate
//! `sonic-connector-host` because its method surface ties together
//! transport-iox handles with health subscription and lifecycle control
//! — concerns this crate intentionally does not own.

#![warn(missing_docs)]

pub mod channel;
pub mod envelope;
pub mod factory;
mod now;
pub mod raw;

pub use channel::{ChannelReader, ChannelWriter, RecvEnvelope};
pub use envelope::ConnectorEnvelope;
pub use factory::ServiceFactory;
pub use raw::{RawChannelReader, RawChannelWriter, RawSample, RawSendOutcome};
