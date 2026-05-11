//! Per-channel routing registry used by the gateway's cycle loop.
//! `REQ_0328`.
//!
//! The registry maps each open [`ChannelDescriptor`] (plugin-side
//! call to `Connector::create_writer` / `create_reader`) to its
//! [`EthercatRouting`] + direction so the cycle loop can iterate it
//! once per cycle, draining outbound bridges into PDI bytes and
//! repopulating inbound iceoryx2 services from PDI bytes.
//!
//! Two properties drive the data structure choice:
//!
//! 1. **Stable insertion-order iteration** — the cycle loop must
//!    visit channels in a deterministic order so test fixtures
//!    (and production observability) can predict the dispatch
//!    sequence.
//! 2. **No per-cycle heap allocation** — `Vec::iter()` returns a
//!    `slice::Iter` that allocates nothing. Registration uses
//!    `Vec::push` (which may allocate at construction time only if
//!    capacity is exceeded), but the cycle loop only ever calls
//!    `iter()`. Combined with `with_capacity` at construction, the
//!    steady state is alloc-free per `REQ_0060`.
//!
//! [`ChannelDescriptor`]: sonic_connector_core::ChannelDescriptor

use std::borrow::Cow;
use std::fmt;

use sonic_connector_core::ConnectorError;

use crate::routing::{EthercatRouting, PdoDirection};

/// Gateway-side outbound drain. `REQ_0326`.
///
/// Wraps an iceoryx2 subscriber whose publisher lives on the plugin
/// side. The dispatcher calls [`Self::drain_into`] once per cycle per
/// outbound channel to move the plugin's already-encoded bytes into
/// the per-cycle PDI outputs.
///
/// `try_recv_into` semantics — the returned `Ok(Some(n))` reports the
/// number of payload bytes copied into `dest[..n]`. Returning
/// `Ok(None)` means "no envelope was pending"; the dispatcher moves
/// on. Errors surface as [`ConnectorError`].
pub trait OutboundDrain: Send {
    /// Drain one envelope into `dest`. Implementations should be
    /// non-blocking — the dispatcher calls this in a tight loop.
    fn drain_into(&self, dest: &mut [u8]) -> Result<Option<usize>, ConnectorError>;
}

/// Gateway-side inbound publisher. `REQ_0327`.
///
/// Wraps an iceoryx2 publisher whose subscriber lives on the plugin
/// side. The dispatcher calls [`Self::publish_bytes`] once per cycle
/// per inbound channel after the PDI bit slice has been read out.
pub trait InboundPublish: Send {
    /// Publish `bytes` verbatim on the channel's inbound iceoryx2
    /// service.
    fn publish_bytes(&self, bytes: &[u8]) -> Result<(), ConnectorError>;
}

/// One entry in the [`ChannelRegistry`].
#[derive(Debug)]
pub struct RegisteredChannel {
    /// `ChannelDescriptor::name()` cloned at registration time.
    /// `Cow` so test fixtures can register `&'static` names without
    /// allocating.
    pub descriptor_name: Cow<'static, str>,
    /// The bit-slice routing this channel's payloads land at.
    pub routing: EthercatRouting,
    /// Plugin-side perspective: `Rx` for plugin → gateway →
    /// SubDevice outputs (RxPDO on the bus); `Tx` for SubDevice
    /// inputs → gateway → plugin (TxPDO on the bus).
    pub direction: PdoDirection,
    /// Source of bytes (outbound) or sink of bytes (inbound).
    /// C7a ships this as a stub variant; C7b populates the real
    /// iceoryx2-backed handles inside the gateway-side dispatcher.
    pub binding: ChannelBinding,
}

/// Channel ↔ iceoryx2 binding. Sealed enum opaque to user code; the
/// gateway dispatcher matches on the variant per cycle.
///
/// The trait-object variants carry the gateway-side iceoryx2 port
/// (subscriber for outbound, publisher for inbound) without exposing
/// the channel's user-type `T` or codec `C` — only the byte slice
/// stays in the dispatcher hot path (`REQ_0326`, `REQ_0327`).
#[non_exhaustive]
pub enum ChannelBinding {
    /// Stub variant retained from C7a — used in unit tests that
    /// exercise the registry in isolation and as a transitional
    /// placeholder before a real port is constructed.
    Unbound,
    /// Plugin → gateway path. Holds the gateway-side iceoryx2
    /// subscriber the dispatcher drains into the SubDevice's outputs
    /// PDI slice.
    Outbound(Box<dyn OutboundDrain>),
    /// Gateway → plugin path. Holds the gateway-side iceoryx2
    /// publisher the dispatcher feeds with bytes extracted from the
    /// SubDevice's inputs PDI slice.
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
///
/// Indexes into the registry's backing `Vec`; the cycle loop never
/// uses it (it iterates instead), but tests and observability code
/// can look up entries.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct ChannelHandle(pub usize);

/// Vec-backed channel registry.
///
/// Construct with [`ChannelRegistry::with_capacity`] for the expected
/// channel count; further registrations beyond the initial capacity
/// will reallocate (acceptable at startup, not on the cycle hot
/// path).
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

    /// Construct a registry pre-sized for `capacity` channels. The
    /// hot-path `iter()` never reallocates after this; `register`
    /// only reallocates if the actual channel count exceeds
    /// `capacity`.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            channels: Vec::with_capacity(capacity),
        }
    }

    /// Append a channel. Returns the [`ChannelHandle`] indexing into
    /// the registry.
    pub fn register(
        &mut self,
        descriptor_name: impl Into<Cow<'static, str>>,
        routing: EthercatRouting,
        direction: PdoDirection,
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

    /// Number of registered channels.
    #[must_use]
    pub fn len(&self) -> usize {
        self.channels.len()
    }

    /// `true` when no channels have been registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.channels.is_empty()
    }

    /// Iterate channels in registration order. Allocation-free —
    /// returns `slice::Iter`, no temporary `Vec` constructed.
    pub fn iter(&self) -> std::slice::Iter<'_, RegisteredChannel> {
        self.channels.iter()
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
