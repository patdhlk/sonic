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

use crate::routing::{EthercatRouting, PdoDirection};

/// One entry in the [`ChannelRegistry`].
#[derive(Clone, Debug)]
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

/// Channel ↔ iceoryx2 / bridge binding. Sealed enum opaque to user
/// code; the gateway dispatcher matches on the variant.
///
/// C7a only declares the variants and the stub `Unbound`; the
/// concrete `Outbound` / `Inbound` variants land in C7b alongside
/// the gateway dispatcher.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum ChannelBinding {
    /// Placeholder used by C7a tests and by the registry's stub-
    /// construction helpers. C7b replaces this with concrete
    /// iceoryx2 publisher / subscriber handles.
    Unbound,
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
