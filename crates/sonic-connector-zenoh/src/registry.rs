//! Per-channel routing registry for the Zenoh gateway dispatcher.
//!
//! Mirrors `sonic_connector_ethercat::registry` in shape — the
//! dispatcher iterates this registry to drive outbound and inbound
//! traffic. Zenoh's direction model is binary (`Outbound` / `Inbound`)
//! whereas `EtherCAT` carries the PDO direction (`Rx` / `Tx`); the
//! registry shape is otherwise identical.

use std::borrow::Cow;
use std::fmt;

use sonic_connector_core::ConnectorError;

use crate::routing::ZenohRouting;

/// Plugin-relative direction for one channel binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelDirection {
    /// Plugin → gateway → session. Plugin holds the iceoryx2 publisher
    /// (`ChannelWriter`); gateway holds the iceoryx2 subscriber
    /// (`RawChannelReader`) that drains into `session.publish`.
    Outbound,
    /// Session → gateway → plugin. Gateway holds the iceoryx2 publisher
    /// (`RawChannelWriter`); plugin holds the iceoryx2 subscriber
    /// (`ChannelReader`).
    Inbound,
}

/// Gateway-side outbound drain — wraps an iceoryx2 raw subscriber so
/// the dispatcher can copy plugin-published bytes into a scratch
/// buffer before forwarding to `session.publish`.
pub trait OutboundDrain: Send {
    /// Drain one envelope into `dest`. Implementations should be
    /// non-blocking — the dispatcher calls this in a tight loop.
    /// Returns `Ok(Some(n))` with the number of bytes copied;
    /// `Ok(None)` if no envelope was pending; `Err(...)` on failure.
    fn drain_into(&self, dest: &mut [u8]) -> Result<Option<usize>, ConnectorError>;
}

/// Gateway-side inbound publish — wraps an iceoryx2 raw publisher so
/// session callbacks can republish bytes verbatim on the channel's
/// inbound service.
pub trait InboundPublish: Send + Sync {
    /// Publish `bytes` verbatim. Implementations must be cheap because
    /// session callbacks may invoke this from hot paths.
    fn publish_bytes(&self, bytes: &[u8]) -> Result<(), ConnectorError>;
}

/// Channel ↔ iceoryx2 binding. Sealed enum opaque to user code; the
/// dispatcher matches on the variant per dispatch tick.
pub enum ChannelBinding {
    /// Outbound — gateway drains bytes via the wrapped subscriber.
    Outbound(Box<dyn OutboundDrain>),
    /// Inbound — gateway re-publishes bytes via the wrapped publisher.
    Inbound(Box<dyn InboundPublish>),
}

impl fmt::Debug for ChannelBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Outbound(_) => f.write_str("Outbound(<dyn OutboundDrain>)"),
            Self::Inbound(_) => f.write_str("Inbound(<dyn InboundPublish>)"),
        }
    }
}

/// One entry in the [`ChannelRegistry`].
#[derive(Debug)]
pub struct RegisteredChannel {
    /// `ChannelDescriptor::name()` cloned at registration time.
    /// `Cow` so test fixtures can register `&'static` names without
    /// allocating.
    pub descriptor_name: Cow<'static, str>,
    /// The Zenoh routing for this channel.
    pub routing: ZenohRouting,
    /// Plugin-relative direction.
    pub direction: ChannelDirection,
    /// Source of bytes (outbound) or sink of bytes (inbound).
    pub binding: ChannelBinding,
}

/// Vec-backed channel registry.
///
/// Construct with [`ChannelRegistry::with_capacity`] for the expected
/// channel count; further registrations beyond the initial capacity
/// will reallocate (acceptable at startup, not on the dispatch hot
/// path).
///
/// Iteration is alloc-free — `iter()` returns `slice::Iter` and
/// constructs no temporary `Vec`.
#[derive(Debug, Default)]
pub struct ChannelRegistry {
    entries: Vec<RegisteredChannel>,
}

impl ChannelRegistry {
    /// Construct an empty registry pre-sized for `cap` channels.
    /// The hot-path `iter()` never reallocates after this; `register`
    /// only reallocates if the actual channel count exceeds `cap`.
    #[must_use]
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            entries: Vec::with_capacity(cap),
        }
    }

    /// Append a channel. Returns an error if a channel with the same
    /// `(name, direction)` tuple is already registered.
    pub fn register(
        &mut self,
        name: String,
        routing: ZenohRouting,
        direction: ChannelDirection,
        binding: ChannelBinding,
    ) -> Result<(), ConnectorError> {
        if self
            .entries
            .iter()
            .any(|e| e.descriptor_name == name && e.direction == direction)
        {
            return Err(ConnectorError::InvalidDescriptor(format!(
                "channel '{name}' already registered with direction {direction:?}",
            )));
        }
        self.entries.push(RegisteredChannel {
            descriptor_name: Cow::Owned(name),
            routing,
            direction,
            binding,
        });
        Ok(())
    }

    /// Number of registered channels.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` when no channels have been registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate channels in registration order. Allocation-free —
    /// returns `slice::Iter`, no temporary `Vec` constructed.
    pub fn iter(&self) -> std::slice::Iter<'_, RegisteredChannel> {
        self.entries.iter()
    }
}

impl<'a> IntoIterator for &'a ChannelRegistry {
    type Item = &'a RegisteredChannel;
    type IntoIter = std::slice::Iter<'a, RegisteredChannel>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
