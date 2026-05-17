//! Per-iface channel registry used by the gateway's RX/TX loops.
//! `REQ_0525`.
//!
//! Maps each registered [`ChannelDescriptor`] to its [`CanRouting`]
//! and direction so the dispatcher can iterate matching channels
//! per iface on every RX frame and per TX drain.
//!
//! Two properties drive the data structure choice (same rationale
//! as the EtherCAT registry):
//!
//! 1. **Stable insertion-order iteration** — the dispatcher visits
//!    channels in a deterministic order so test fixtures (and
//!    production observability) can predict dispatch order.
//! 2. **No per-hot-path heap allocation** — `Vec::iter()` returns a
//!    `slice::Iter` that allocates nothing. `Vec::with_capacity` at
//!    construction keeps `register` allocation-free up to the
//!    configured channel count.
//!
//! [`ChannelDescriptor`]: sonic_connector_core::ChannelDescriptor

use std::borrow::Cow;
use std::fmt;

use sonic_connector_core::ConnectorError;

use crate::routing::{CanIface, CanRouting};

/// Direction of a registered channel from the plugin's perspective.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    /// Plugin → bus (writer). Dispatcher's TX drain consumes from
    /// the channel's outbound bridge.
    Outbound,
    /// Bus → plugin (reader). Dispatcher's RX classifier feeds the
    /// inbound bridge.
    Inbound,
}

/// Gateway-side outbound drain. `REQ_0513`.
///
/// Wraps an iceoryx2 raw subscriber whose publisher lives on the
/// plugin side. The dispatcher calls [`Self::drain_into`] each TX
/// iteration to pull the next codec-encoded envelope into a CAN
/// frame's payload bytes.
///
/// Only `Send` (not `Sync`) — iceoryx2 ports are not `Sync`. The
/// dispatcher never holds a `&Box<dyn OutboundDrain>` across an
/// `.await` point; it collects bytes into an owned vector inside
/// the registry-lock scope, then drops the lock before awaiting any
/// driver send.
pub trait OutboundDrain: Send {
    /// Drain one envelope into `dest`. Returns `Ok(Some(n))` with
    /// the byte count copied; `Ok(None)` when nothing is pending.
    fn drain_into(&self, dest: &mut [u8]) -> Result<Option<usize>, ConnectorError>;
}

/// Gateway-side inbound publisher. `REQ_0514`.
///
/// Wraps an iceoryx2 raw publisher whose subscriber lives on the
/// plugin side. The dispatcher calls [`Self::publish_bytes`] for
/// each matching reader on every inbound CAN frame.
///
/// Same `Send`-only rationale as [`OutboundDrain`].
pub trait InboundPublish: Send {
    /// Publish `bytes` verbatim on the channel's inbound iceoryx2
    /// service.
    fn publish_bytes(&self, bytes: &[u8]) -> Result<(), ConnectorError>;
}

/// One entry in the [`ChannelRegistry`].
pub struct RegisteredChannel {
    /// `ChannelDescriptor::name()` cloned at registration time.
    pub descriptor_name: Cow<'static, str>,
    /// The CAN routing this channel's frames map to.
    pub routing: CanRouting,
    /// Outbound (writer) or inbound (reader).
    pub direction: Direction,
    /// Source of bytes (outbound) or sink of bytes (inbound).
    pub binding: ChannelBinding,
}

impl fmt::Debug for RegisteredChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RegisteredChannel")
            .field("descriptor_name", &self.descriptor_name)
            .field("routing", &self.routing)
            .field("direction", &self.direction)
            .field("binding", &self.binding)
            .finish()
    }
}

/// Channel ↔ iceoryx2 binding. Sealed enum.
#[non_exhaustive]
pub enum ChannelBinding {
    /// Stub variant — used in unit tests that exercise the registry
    /// in isolation.
    Unbound,
    /// Plugin → gateway path.
    Outbound(Box<dyn OutboundDrain>),
    /// Gateway → plugin path.
    Inbound(Box<dyn InboundPublish>),
}

impl fmt::Debug for ChannelBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unbound => f.write_str("Unbound"),
            Self::Outbound(_) => f.write_str("Outbound(<dyn OutboundDrain>)"),
            Self::Inbound(_) => f.write_str("Inbound(<dyn InboundPublish>)"),
        }
    }
}

/// Opaque handle returned from [`ChannelRegistry::register`].
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct ChannelHandle(pub usize);

/// Vec-backed registry. Iteration is stable in insertion order and
/// allocation-free.
#[derive(Debug, Default)]
pub struct ChannelRegistry {
    channels: Vec<RegisteredChannel>,
}

impl ChannelRegistry {
    /// Construct an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct with pre-allocated capacity. `register` is
    /// alloc-free until `capacity` is exceeded.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            channels: Vec::with_capacity(capacity),
        }
    }

    /// Append a channel and return its handle.
    pub fn register(
        &mut self,
        descriptor_name: impl Into<Cow<'static, str>>,
        routing: CanRouting,
        direction: Direction,
        binding: ChannelBinding,
    ) -> ChannelHandle {
        let handle = ChannelHandle(self.channels.len());
        self.channels.push(RegisteredChannel {
            descriptor_name: descriptor_name.into(),
            routing,
            direction,
            binding,
        });
        handle
    }

    /// Number of registered channels (total across all ifaces).
    #[must_use]
    pub fn len(&self) -> usize {
        self.channels.len()
    }

    /// `true` when no channels have been registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.channels.is_empty()
    }

    /// Iterate channels in registration order. Alloc-free.
    pub fn iter(&self) -> std::slice::Iter<'_, RegisteredChannel> {
        self.channels.iter()
    }

    /// Iterate channels bound to the specified iface, filtered to
    /// the given direction. Alloc-free; returns an iterator adaptor
    /// over the underlying slice.
    pub fn iter_iface_direction<'a>(
        &'a self,
        iface: &'a CanIface,
        direction: Direction,
    ) -> impl Iterator<Item = &'a RegisteredChannel> {
        self.channels
            .iter()
            .filter(move |c| c.routing.iface == *iface && c.direction == direction)
    }

    /// Borrow a single channel by handle.
    #[must_use]
    pub fn get(&self, handle: ChannelHandle) -> Option<&RegisteredChannel> {
        self.channels.get(handle.0)
    }
}

impl<'a> IntoIterator for &'a ChannelRegistry {
    type Item = &'a RegisteredChannel;
    type IntoIter = std::slice::Iter<'a, RegisteredChannel>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::{CanFrameKind, CanId};

    #[test]
    fn iter_iface_direction_filters_correctly() {
        let a = CanIface::new("vcan0").unwrap();
        let b = CanIface::new("vcan1").unwrap();
        let routing_a_in = CanRouting::new(
            a,
            CanId::standard(0x100).unwrap(),
            0x7FF,
            CanFrameKind::Classical,
        );
        let routing_b_in = CanRouting::new(
            b,
            CanId::standard(0x200).unwrap(),
            0x7FF,
            CanFrameKind::Classical,
        );
        let routing_a_out = CanRouting::new(
            a,
            CanId::standard(0x101).unwrap(),
            0x7FF,
            CanFrameKind::Classical,
        );
        let mut r = ChannelRegistry::with_capacity(4);
        r.register(
            "a_in",
            routing_a_in,
            Direction::Inbound,
            ChannelBinding::Unbound,
        );
        r.register(
            "b_in",
            routing_b_in,
            Direction::Inbound,
            ChannelBinding::Unbound,
        );
        r.register(
            "a_out",
            routing_a_out,
            Direction::Outbound,
            ChannelBinding::Unbound,
        );

        let a_inbound: Vec<&str> = r
            .iter_iface_direction(&a, Direction::Inbound)
            .map(|c| c.descriptor_name.as_ref())
            .collect();
        assert_eq!(a_inbound, vec!["a_in"]);

        let a_outbound: Vec<&str> = r
            .iter_iface_direction(&a, Direction::Outbound)
            .map(|c| c.descriptor_name.as_ref())
            .collect();
        assert_eq!(a_outbound, vec!["a_out"]);

        let b_inbound: Vec<&str> = r
            .iter_iface_direction(&b, Direction::Inbound)
            .map(|c| c.descriptor_name.as_ref())
            .collect();
        assert_eq!(b_inbound, vec!["b_in"]);
    }
}
