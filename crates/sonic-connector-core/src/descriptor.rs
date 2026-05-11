//! [`ChannelDescriptor`] — typed routing + per-channel max payload size.
//! `REQ_0201`, `REQ_0221`.

use std::borrow::Cow;

use crate::{ConnectorError, Routing};

/// Describes one logical channel.
///
/// A `ChannelDescriptor` carries a name (used for iceoryx2 service
/// naming and tracing), the connector-specific routing struct `R`, and
/// the maximum payload size `N` as a compile-time const generic.
///
/// `N` propagates into [`ChannelWriter`] / [`ChannelReader`] via the
/// `Connector::create_writer<T, const N: usize>` /
/// `Connector::create_reader<T, const N: usize>` methods so the channel's
/// envelope buffer is sized once at compile time and never reallocated
/// (`REQ_0201`, `REQ_0205`). Mismatched `N` values between writer and
/// reader cannot type-check.
///
/// [`ChannelWriter`]: # "lives in sonic-connector-transport-iox (BB_0002)"
/// [`ChannelReader`]: # "lives in sonic-connector-transport-iox (BB_0002)"
#[derive(Clone, Debug)]
pub struct ChannelDescriptor<R, const N: usize>
where
    R: Routing,
{
    name: Cow<'static, str>,
    routing: R,
}

impl<R, const N: usize> ChannelDescriptor<R, N>
where
    R: Routing,
{
    /// Construct a descriptor.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectorError::InvalidDescriptor`] when `name` is
    /// empty. Other validations (length cap, character set) live in
    /// `sonic-connector-transport-iox` because they are tied to
    /// iceoryx2's service-name constraints.
    pub fn new(name: impl Into<Cow<'static, str>>, routing: R) -> Result<Self, ConnectorError> {
        let name = name.into();
        if name.is_empty() {
            return Err(ConnectorError::InvalidDescriptor(
                "channel name must not be empty".into(),
            ));
        }
        Ok(Self { name, routing })
    }

    /// Borrow the channel's logical name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Borrow the connector-specific routing struct.
    #[must_use]
    pub const fn routing(&self) -> &R {
        &self.routing
    }

    /// Compile-time maximum payload size for this channel — the `N`
    /// const generic. Returned as a `usize` for convenience.
    #[must_use]
    pub const fn max_payload_size(&self) -> usize {
        N
    }
}
