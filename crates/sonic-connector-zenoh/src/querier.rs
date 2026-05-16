//! [`ZenohQuerier`] — plugin-side query-initiation handle (`REQ_0420`,
//! `REQ_0421`). Constructed by [`crate::ZenohConnector::create_querier`]
//! (wired in Z3e).

use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use sonic_connector_core::{ConnectorError, PayloadCodec};
use sonic_connector_transport_iox::{RawChannelReader, RawChannelWriter};

use crate::registry::QueryId;
use crate::session::FrameKind;

/// Mint a fresh [`QueryId`] from a process-global counter.
///
/// The first 8 bytes are a monotonic `u64`; the remaining 24 bytes are
/// zero. The id is unique within a process for the lifetime of the
/// program (the counter is `AtomicU64`, so wraparound after 2^64 calls
/// is theoretically possible but practically irrelevant).
#[must_use]
pub fn mint_query_id() -> QueryId {
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut bytes = [0u8; 32];
    bytes[..8].copy_from_slice(&n.to_be_bytes());
    QueryId(bytes)
}

/// Reusable [`QueryId`] minter — same counter shape as
/// [`mint_query_id`] but per-instance, so tests get a deterministic
/// counter and don't interfere with the global one.
#[derive(Debug, Default)]
pub struct ZeroedMinter {
    counter: AtomicU64,
}

impl ZeroedMinter {
    /// Create a fresh minter starting at counter = 1.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            counter: AtomicU64::new(1),
        }
    }

    /// Mint the next [`QueryId`].
    pub fn next(&self) -> QueryId {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        let mut bytes = [0u8; 32];
        bytes[..8].copy_from_slice(&n.to_be_bytes());
        QueryId(bytes)
    }
}

/// One event observed by [`ZenohQuerier::try_recv`].
#[derive(Debug, Clone)]
pub enum QuerierEvent<R> {
    /// One reply chunk decoded from the upstream stream (`0x01`
    /// framed).
    Reply {
        /// The query this reply belongs to.
        id: QueryId,
        /// The decoded reply value.
        value: R,
    },
    /// Upstream queryable terminated the stream normally (`0x02`).
    EndOfStream {
        /// The query the stream finalised.
        id: QueryId,
    },
    /// Gateway-synthetic timeout terminator (`0x03`) — the query
    /// expired before the upstream finished.
    Timeout {
        /// The query that timed out.
        id: QueryId,
    },
}

/// Plugin-side query-initiation handle (`REQ_0420`).
///
/// `Q` is the request type; `R` is the reply type. Both flow through
/// the connector's [`PayloadCodec`] (`REQ_0427`).
pub struct ZenohQuerier<Q, R, C, const N: usize>
where
    C: PayloadCodec,
{
    writer: RawChannelWriter<N>,
    reader: RawChannelReader<N>,
    codec: C,
    scratch: Vec<u8>,
    _ty: PhantomData<fn() -> (Q, R)>,
}

impl<Q, R, C, const N: usize> ZenohQuerier<Q, R, C, N>
where
    Q: serde::Serialize,
    R: serde::de::DeserializeOwned,
    C: PayloadCodec,
{
    /// Construct a querier from raw iox handles. Called only by the
    /// connector's `create_querier` impl.
    pub(crate) fn new(writer: RawChannelWriter<N>, reader: RawChannelReader<N>, codec: C) -> Self {
        Self {
            writer,
            reader,
            codec,
            scratch: vec![0u8; N],
            _ty: PhantomData,
        }
    }

    /// Issue a query against the configured key expression. Returns
    /// the freshly minted [`QueryId`].
    ///
    /// The envelope's `reserved` header word is stamped with `0`,
    /// telling the gateway to fall back to the connector's default
    /// `query_timeout` (`REQ_0425`). Use [`Self::send_with_timeout`]
    /// for per-call overrides.
    ///
    /// # Errors
    /// Returns [`ConnectorError::Codec`] on encode failure;
    /// [`ConnectorError::PayloadOverflow`] if the encoded `Q` exceeds
    /// `N`; [`ConnectorError::Stack`] for iox failures.
    pub fn send(&mut self, q: &Q) -> Result<QueryId, ConnectorError> {
        // `Duration::ZERO` -> `reserved = 0` -> gateway uses default.
        self.send_inner(q, Duration::ZERO)
    }

    /// Issue a query with a per-call timeout override (`REQ_0425`).
    /// The timeout in milliseconds is stamped into the envelope's
    /// `reserved` header word; the gateway reads it and uses
    /// `tokio::time::timeout` to enforce expiry, emitting a synthetic
    /// `[0x03]` (`FrameKind::Timeout`) frame on the reply path when
    /// the budget elapses.
    ///
    /// Pass `Duration::ZERO` (or just use [`Self::send`]) to fall back
    /// to the connector's default `query_timeout`.
    ///
    /// # Errors
    /// Returns [`ConnectorError::Codec`] on encode failure;
    /// [`ConnectorError::PayloadOverflow`] if the encoded `Q` exceeds
    /// `N`; [`ConnectorError::Stack`] for iox failures.
    pub fn send_with_timeout(
        &mut self,
        q: &Q,
        timeout: Duration,
    ) -> Result<QueryId, ConnectorError> {
        self.send_inner(q, timeout)
    }

    fn send_inner(&mut self, q: &Q, timeout: Duration) -> Result<QueryId, ConnectorError> {
        let id = mint_query_id();
        let written = self.codec.encode(q, &mut self.scratch)?;
        // Saturate at `u32::MAX` ms (~49.7 days) — anything that
        // overflows is clamped rather than silently wrapping.
        let timeout_ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
        self.writer
            .send_raw_bytes_v2(&self.scratch[..written], id.0, timeout_ms)?;
        Ok(id)
    }

    /// Try to receive one event from the reply stream. Returns
    /// `Ok(None)` if no envelope is pending; `Ok(Some(event))`
    /// otherwise. The event encodes whether the byte is a data chunk,
    /// end-of-stream, or timeout terminator.
    ///
    /// # Errors
    /// Returns [`ConnectorError::Codec`] on decode failure;
    /// [`ConnectorError::Stack`] for iox failures or an unrecognised
    /// reply-frame discriminator.
    pub fn try_recv(&mut self) -> Result<Option<QuerierEvent<R>>, ConnectorError> {
        let Some(sample) = self.reader.try_recv_into(&mut self.scratch)? else {
            return Ok(None);
        };
        let id = QueryId(sample.correlation_id);
        let len = sample.payload_len;
        if len == 0 {
            return Ok(None);
        }
        let discriminator = self.scratch[0];
        match FrameKind::from_byte(discriminator) {
            Some(FrameKind::Data) => {
                let value: R = self.codec.decode(&self.scratch[1..len])?;
                Ok(Some(QuerierEvent::Reply { id, value }))
            }
            Some(FrameKind::EndOfStream) => Ok(Some(QuerierEvent::EndOfStream { id })),
            Some(FrameKind::Timeout) => Ok(Some(QuerierEvent::Timeout { id })),
            None => Err(ConnectorError::stack(InvalidReplyFrame(discriminator))),
        }
    }
}

#[derive(Debug)]
struct InvalidReplyFrame(u8);

impl core::fmt::Display for InvalidReplyFrame {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "invalid reply-frame discriminator: 0x{:02X}", self.0)
    }
}

impl std::error::Error for InvalidReplyFrame {}
