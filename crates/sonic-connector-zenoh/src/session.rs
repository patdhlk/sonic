//! Abstraction trait over real and mock Zenoh sessions, plus the
//! Zenoh-private 1-byte reply framing helpers.
//!
//! The trait shape covers BOTH pub/sub and query operations so that
//! `ZenohConnector`, `ZenohGateway`, and the query handles (added in
//! later stages) can monomorphise over either back-end without
//! re-defining the surface. Z1 lands the trait + frame helpers only;
//! `MockZenohSession` (in `mock.rs`) implements pub/sub fully and
//! stubs out query operations as `NotImplemented` until Z3.

use std::time::Duration;

use crate::routing::ZenohRouting;

/// Callback type for pub/sub sinks and query reply listeners.
pub type PayloadSink = Box<dyn Fn(&[u8]) + Send + Sync + 'static>;

/// Callback type for query `on_done` / subscription tear-down.
pub type DoneCallback = Box<dyn FnOnce() + Send + 'static>;

/// Callback type for incoming queries (payload + replier).
pub type QuerySink = Box<dyn Fn(&[u8], QueryReplier) + Send + Sync + 'static>;

/// The session's observable connection state. Maps to
/// `ConnectorHealth` variants in the gateway layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionState {
    /// `zenoh::open` is awaiting peers / router.
    Connecting,
    /// The session is alive and has at least one observable primitive
    /// declared.
    Alive,
    /// The session has closed; carries a human-readable reason.
    Closed {
        /// Reason for closure (free-form string, surfaced through
        /// `HealthEvent` payloads).
        reason: String,
    },
}

/// One of the three Zenoh-private reply-framing byte values per
/// `REQ_0424` / `ADR_0043`. These byte values are part of the wire
/// contract — do not re-number without updating the spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    /// Data chunk: discriminator + codec-encoded `R`.
    Data,
    /// End-of-stream terminator: discriminator only, no body.
    EndOfStream,
    /// Gateway-synthetic timeout terminator: discriminator only.
    Timeout,
}

impl FrameKind {
    /// Return the wire-format discriminator byte for this variant.
    #[must_use]
    pub const fn discriminator(self) -> u8 {
        match self {
            Self::Data => 0x01,
            Self::EndOfStream => 0x02,
            Self::Timeout => 0x03,
        }
    }

    /// Attempt to parse a [`FrameKind`] from a raw discriminator byte.
    /// Returns `None` for unrecognised values.
    #[must_use]
    pub const fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x01 => Some(Self::Data),
            0x02 => Some(Self::EndOfStream),
            0x03 => Some(Self::Timeout),
            _ => None,
        }
    }
}

/// Errors surfaced from reply-frame encode / decode.
#[derive(Debug, thiserror::Error)]
pub enum ReplyFrameError {
    /// The output buffer was too small to hold the framed payload.
    #[error("reply-frame encode: buffer too small (need {need}, have {have})")]
    BufferTooSmall {
        /// Required buffer length.
        need: usize,
        /// Actual buffer length.
        have: usize,
    },
    /// The input was an empty envelope (no discriminator byte).
    #[error("reply-frame decode: empty envelope")]
    EmptyEnvelope,
    /// The discriminator byte did not match any known `FrameKind`.
    #[error("reply-frame decode: unknown discriminator byte 0x{0:02X}")]
    UnknownKind(u8),
}

/// Decoded reply frame: a [`FrameKind`] discriminator + a body slice.
#[derive(Debug, Clone, Copy)]
pub struct ReplyFrame<'a> {
    kind: FrameKind,
    body: &'a [u8],
}

impl<'a> ReplyFrame<'a> {
    /// Return the [`FrameKind`] discriminator for this frame.
    #[must_use]
    pub const fn kind(&self) -> FrameKind {
        self.kind
    }

    /// Return the body bytes following the discriminator (may be empty).
    #[must_use]
    pub const fn body(&self) -> &'a [u8] {
        self.body
    }

    /// Encode a data-chunk frame: `[0x01, body...]`. Returns the number
    /// of bytes written into `out`.
    pub fn encode_data(body: &[u8], out: &mut [u8]) -> Result<usize, ReplyFrameError> {
        let need = 1 + body.len();
        if out.len() < need {
            return Err(ReplyFrameError::BufferTooSmall {
                need,
                have: out.len(),
            });
        }
        out[0] = FrameKind::Data.discriminator();
        out[1..need].copy_from_slice(body);
        Ok(need)
    }

    /// Encode a 1-byte end-of-stream terminator.
    pub fn encode_end_of_stream(out: &mut [u8]) -> Result<usize, ReplyFrameError> {
        if out.is_empty() {
            return Err(ReplyFrameError::BufferTooSmall { need: 1, have: 0 });
        }
        out[0] = FrameKind::EndOfStream.discriminator();
        Ok(1)
    }

    /// Encode a 1-byte gateway-synthetic timeout terminator.
    pub fn encode_timeout(out: &mut [u8]) -> Result<usize, ReplyFrameError> {
        if out.is_empty() {
            return Err(ReplyFrameError::BufferTooSmall { need: 1, have: 0 });
        }
        out[0] = FrameKind::Timeout.discriminator();
        Ok(1)
    }

    /// Decode a reply envelope payload: first byte is the discriminator,
    /// remainder is the body (empty for [`FrameKind::EndOfStream`] /
    /// [`FrameKind::Timeout`]).
    pub fn decode(envelope: &'a [u8]) -> Result<Self, ReplyFrameError> {
        if envelope.is_empty() {
            return Err(ReplyFrameError::EmptyEnvelope);
        }
        let kind = FrameKind::from_byte(envelope[0])
            .ok_or(ReplyFrameError::UnknownKind(envelope[0]))?;
        Ok(Self {
            kind,
            body: &envelope[1..],
        })
    }
}

/// Abstraction over real and mock Zenoh sessions.
///
/// The trait surface covers pub/sub *and* queries; concrete
/// implementations may stub out query operations (e.g.
/// `MockZenohSession` in Z1) and fill them in later (Z3).
///
/// All methods are non-async by design — the real `zenoh::Session`
/// runs its callbacks on the gateway's tokio runtime, but the trait
/// surface itself stays sync to keep the unit-test path mock-friendly.
pub trait ZenohSessionLike: Send + Sync + 'static {
    /// Current observable session state. Polled by the gateway to
    /// transition `ConnectorHealth`.
    fn state(&self) -> SessionState;

    /// Publish a sample on the given routing's key expression.
    fn publish(&self, routing: &ZenohRouting, payload: &[u8]) -> Result<(), SessionError>;

    /// Subscribe to samples matching the routing's key expression.
    /// Implementations return an opaque subscription handle that the
    /// caller drops to unsubscribe.
    fn subscribe(
        &self,
        routing: &ZenohRouting,
        sink: PayloadSink,
    ) -> Result<SubscriptionHandle, SessionError>;

    /// Issue a query against the given routing's key expression with
    /// the given request payload and timeout. The `on_reply` callback
    /// fires once per reply chunk; the `on_done` callback fires once
    /// when the upstream stream is finalised. Z1 implementations are
    /// permitted to stub this out as `NotImplemented` — Z3 lands the
    /// real behavior.
    fn query(
        &self,
        routing: &ZenohRouting,
        payload: &[u8],
        timeout: Duration,
        on_reply: PayloadSink,
        on_done: DoneCallback,
    ) -> Result<(), SessionError>;

    /// Declare a queryable on the routing's key expression. The
    /// `on_query` callback receives an incoming query's payload + a
    /// reply sender. Z1 implementations are permitted to stub this
    /// out — Z3 lands the real behavior.
    fn declare_queryable(
        &self,
        routing: &ZenohRouting,
        on_query: QuerySink,
    ) -> Result<QueryableHandle, SessionError>;
}

/// Opaque subscription handle. Dropping it tears down the subscription.
pub struct SubscriptionHandle(#[allow(dead_code)] pub(crate) Box<dyn std::any::Any + Send + Sync>);

impl std::fmt::Debug for SubscriptionHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubscriptionHandle").finish_non_exhaustive()
    }
}

/// Opaque queryable handle. Dropping it tears down the queryable.
pub struct QueryableHandle(#[allow(dead_code)] pub(crate) Box<dyn std::any::Any + Send + Sync>);

impl std::fmt::Debug for QueryableHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueryableHandle").finish_non_exhaustive()
    }
}

/// Replier handed to a queryable's `on_query` callback. `reply` sends
/// one reply chunk; `terminate` finalises the stream.
pub struct QueryReplier {
    pub(crate) reply: PayloadSink,
    pub(crate) terminate: DoneCallback,
}

impl std::fmt::Debug for QueryReplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueryReplier").finish_non_exhaustive()
    }
}

impl QueryReplier {
    /// Send one reply chunk back to the querier.
    pub fn reply(&self, body: &[u8]) {
        (self.reply)(body);
    }

    /// Finalise the reply stream. After this call, no further `reply`
    /// invocations should be made (callers that try will be ignored or
    /// trip a debug assertion, at the implementation's discretion).
    pub fn terminate(self) {
        (self.terminate)();
    }
}

/// Errors surfaced from [`ZenohSessionLike`] operations.
///
/// These are distinct from `OutboundError` / `InboundOutcome` (which
/// are bridge-level) so the gateway layer can disambiguate session-side
/// failures from bridge-side ones.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// The session is not in a state that accepts the operation
    /// (e.g. `publish` while `Closed`).
    #[error("session not alive: {reason}")]
    NotAlive {
        /// Reason text from the session-state snapshot.
        reason: String,
    },
    /// A query timed out before the upstream peer terminated the
    /// stream. The gateway surfaces this as a synthetic `0x03`
    /// terminator on the reply path (`REQ_0425`).
    #[error("query timed out after {after_ms} ms")]
    QueryTimeout {
        /// Elapsed milliseconds before the timeout fired.
        after_ms: u64,
    },
    /// An operation that has not yet been implemented (Z1 stub for
    /// query operations on `MockZenohSession`).
    #[error("operation not yet implemented: {0}")]
    NotImplemented(&'static str),
}
