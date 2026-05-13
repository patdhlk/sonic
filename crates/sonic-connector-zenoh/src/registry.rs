//! Per-channel routing registry for the Zenoh gateway dispatcher.
//!
//! Mirrors `sonic_connector_ethercat::registry` in shape — the
//! dispatcher iterates this registry to drive outbound and inbound
//! traffic. Zenoh's direction model is binary (`Outbound` / `Inbound`)
//! whereas `EtherCAT` carries the PDO direction (`Rx` / `Tx`); the
//! registry shape is otherwise identical.

use std::borrow::Cow;
use std::fmt;
use std::sync::Arc;

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
    /// Plugin → gateway → `session.query`. Gateway drains envelope
    /// (`correlation_id` = the `QueryId`, payload = encoded `Q`).
    QuerierOut,
    /// Gateway → plugin on the querier's reply path. Gateway publishes
    /// framed reply bytes (`correlation_id` = the `QueryId`, `payload[0]`
    /// = 0x01 data / 0x02 `EoS` / 0x03 timeout).
    QuerierReplyIn,
    /// Gateway → plugin on the queryable's query-receive path. Gateway
    /// publishes `Q` bytes when the upstream session delivers a query
    /// (`correlation_id` = gateway-minted `QueryId`).
    QueryableQueryIn,
    /// Plugin → gateway on the queryable's reply path. Gateway drains
    /// framed reply bytes (`correlation_id` = the `QueryId`, `payload[0]`
    /// = 0x01 data / 0x02 `EoS`).
    QueryableReplyOut,
}

/// A query's correlation identifier — the 32-byte `correlation_id`
/// field of `ConnectorEnvelope` (`REQ_0204`, `REQ_0421`). Reused
/// verbatim as the gateway-side key for the `QueryReplier` map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QueryId(pub [u8; 32]);

impl QueryId {
    /// Borrow the underlying 32-byte array.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl From<[u8; 32]> for QueryId {
    fn from(value: [u8; 32]) -> Self {
        Self(value)
    }
}

impl From<QueryId> for [u8; 32] {
    fn from(value: QueryId) -> Self {
        value.0
    }
}

/// Gateway-side outbound drain — wraps an iceoryx2 raw subscriber so
/// the dispatcher can copy plugin-published bytes into a scratch
/// buffer before forwarding to `session.publish`.
///
/// `Send + Sync` so the dispatcher can hold the drain behind an
/// `Arc<dyn ...>` (the snapshot pattern in `drain_outbound_once`).
pub trait OutboundDrain: Send + Sync {
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

/// Gateway-side drain for the querier's query-out path.
///
/// Like [`OutboundDrain`] but also returns the envelope's
/// `correlation_id` (= the [`QueryId`] minted by `ZenohQuerier::send`)
/// and the envelope's `reserved` header word (per-call timeout
/// override in milliseconds; `0` = use connector default).
///
/// `Send + Sync` so the dispatcher can hold the drain behind an
/// `Arc<dyn ...>` (the snapshot pattern in `drain_outbound_once`).
pub trait QuerierDrain: Send + Sync {
    /// Drain one query envelope into `dest`. Returns
    /// `Ok(Some((id, n, reserved)))` with the [`QueryId`] from the
    /// envelope's `correlation_id`, the number of payload bytes copied,
    /// and the envelope's `reserved` header word (`0` = use connector
    /// default; non-zero = per-call timeout in milliseconds —
    /// `REQ_0425`). `Ok(None)` if no envelope was pending.
    fn drain_query(
        &self,
        dest: &mut [u8],
    ) -> Result<Option<(QueryId, usize, u32)>, ConnectorError>;
}

/// Gateway-side drain for the queryable's reply-out path.
///
/// Like [`OutboundDrain`] but returns the envelope's `correlation_id`
/// (= the [`QueryId`] the gateway minted on query-receive) so the
/// dispatcher can look up the matching upstream `QueryReplier`.
///
/// `Send + Sync` so the dispatcher can hold the drain behind an
/// `Arc<dyn ...>` (the snapshot pattern in `drain_outbound_once`).
pub trait ReplyDrain: Send + Sync {
    /// Drain one reply envelope into `dest`. Returns `Ok(Some((id, n)))`
    /// with the [`QueryId`] from the envelope's `correlation_id` field
    /// and the number of payload bytes copied. `Ok(None)` if no
    /// envelope was pending.
    fn drain_reply(
        &self,
        dest: &mut [u8],
    ) -> Result<Option<(QueryId, usize)>, ConnectorError>;
}

/// Gateway-side publish with an explicit `correlation_id`.
///
/// Used for both [`ChannelBinding::QuerierReplyIn`] (stamping the
/// original [`QueryId`] on each reply chunk) and
/// [`ChannelBinding::QueryableQueryIn`] (stamping the gateway-minted
/// [`QueryId`] on each delivered query).
pub trait CorrelatedPublish: Send + Sync {
    /// Publish `bytes` verbatim, stamping the envelope's
    /// `correlation_id` field with `id`.
    fn publish_with_correlation(
        &self,
        id: QueryId,
        bytes: &[u8],
    ) -> Result<(), ConnectorError>;
}

/// Channel ↔ iceoryx2 binding. Sealed enum opaque to user code; the
/// dispatcher matches on the variant per dispatch tick.
///
/// Drain-side variants (`Outbound`, `QuerierOut`, `QueryableReplyOut`)
/// hold their dyn trait objects behind `Arc` so the async dispatcher
/// (Z4a) can snapshot-clone them out of the registry lock before
/// awaiting on the session.
pub enum ChannelBinding {
    /// Outbound — gateway drains bytes via the wrapped subscriber.
    Outbound(Arc<dyn OutboundDrain>),
    /// Inbound — gateway re-publishes bytes via the wrapped publisher.
    Inbound(Box<dyn InboundPublish>),
    /// Querier-side query-out — gateway drains (id, bytes) and feeds
    /// `session.query`.
    QuerierOut(Arc<dyn QuerierDrain>),
    /// Querier-side reply-in — gateway publishes framed reply chunks
    /// keyed by [`QueryId`].
    QuerierReplyIn(Box<dyn CorrelatedPublish>),
    /// Queryable-side query-in — gateway publishes incoming queries
    /// keyed by a freshly-minted [`QueryId`].
    QueryableQueryIn(Box<dyn CorrelatedPublish>),
    /// Queryable-side reply-out — gateway drains (id, framed bytes)
    /// and forwards via the upstream `QueryReplier`.
    QueryableReplyOut(Arc<dyn ReplyDrain>),
}

impl fmt::Debug for ChannelBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Outbound(_) => f.write_str("Outbound(<dyn OutboundDrain>)"),
            Self::Inbound(_) => f.write_str("Inbound(<dyn InboundPublish>)"),
            Self::QuerierOut(_) => f.write_str("QuerierOut(<dyn QuerierDrain>)"),
            Self::QuerierReplyIn(_) => {
                f.write_str("QuerierReplyIn(<dyn CorrelatedPublish>)")
            }
            Self::QueryableQueryIn(_) => {
                f.write_str("QueryableQueryIn(<dyn CorrelatedPublish>)")
            }
            Self::QueryableReplyOut(_) => {
                f.write_str("QueryableReplyOut(<dyn ReplyDrain>)")
            }
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
