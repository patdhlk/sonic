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
    default_timeout: Duration,
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
    pub(crate) fn new(
        writer: RawChannelWriter<N>,
        reader: RawChannelReader<N>,
        codec: C,
        default_timeout: Duration,
    ) -> Self {
        Self {
            writer,
            reader,
            codec,
            default_timeout,
            scratch: vec![0u8; N],
            _ty: PhantomData,
        }
    }

    /// Issue a query against the configured key expression with the
    /// connector's default timeout. Returns the freshly minted
    /// [`QueryId`].
    ///
    /// # Errors
    /// Returns [`ConnectorError::Codec`] on encode failure;
    /// [`ConnectorError::PayloadOverflow`] if the encoded `Q` exceeds
    /// `N`; [`ConnectorError::Stack`] for iox failures.
    pub fn send(&mut self, q: &Q) -> Result<QueryId, ConnectorError> {
        self.send_inner(q, self.default_timeout)
    }

    /// Like [`Self::send`] but with a per-call timeout override
    /// (`REQ_0425`). The timeout is honoured by the gateway, not by
    /// the plugin — this method just stamps it on the outgoing
    /// envelope for the gateway to read. Z3's first cut uses the
    /// connector's default timeout for ALL queriers; the per-call
    /// override is wired in Z4 alongside the real session integration.
    pub fn send_with_timeout(
        &mut self,
        q: &Q,
        _timeout: Duration,
    ) -> Result<QueryId, ConnectorError> {
        // Z3: timeout is not yet wire-propagated per query. Uses
        // default for now. Z4 will plumb the timeout through the
        // envelope's reserved header word or via a sidecar metadata
        // channel.
        self.send_inner(q, self.default_timeout)
    }

    fn send_inner(&mut self, q: &Q, _timeout: Duration) -> Result<QueryId, ConnectorError> {
        let id = mint_query_id();
        let written = self.codec.encode(q, &mut self.scratch)?;
        self.writer
            .send_raw_bytes(&self.scratch[..written], id.0)?;
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
