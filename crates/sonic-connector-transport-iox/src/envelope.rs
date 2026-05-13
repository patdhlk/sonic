//! [`ConnectorEnvelope`] — POD wire format used by every connector
//! channel. `REQ_0200`, `REQ_0202`, `REQ_0203`, `REQ_0204`.

use iceoryx2::prelude::ZeroCopySend;

/// 32-byte correlation id carried end-to-end (`REQ_0204`). The framework
/// does not interpret these bytes — application layers may.
pub type CorrelationId = [u8; 32];

/// On-wire envelope: a fixed POD header followed by an inline payload
/// buffer of compile-time size `N`.
///
/// `#[repr(C)]` + `ZeroCopySend` make this safe to publish via
/// iceoryx2's loan path. Every field is plain-old-data; the struct is
/// `Copy` for convenience in tests, but production sends use
/// [`iceoryx2::port::publisher::Publisher::loan_uninit`] to avoid the
/// `Copy`-induced stack/SHM round-trip (`REQ_0205`).
///
/// Field order is chosen so the header is naturally aligned without
/// padding on 64-bit targets: `u64 / u64 / [u8;32] / u32 / u32` packs to
/// 56 bytes, and the trailing `[u8; N]` starts at offset 56 (aligned to
/// 1 byte, no padding regardless of `N`).
#[repr(C)]
#[derive(Clone, Copy, Debug, ZeroCopySend)]
pub struct ConnectorEnvelope<const N: usize> {
    /// Per-(publisher, channel) strictly monotonically increasing
    /// counter starting at zero. `REQ_0202`.
    pub sequence_number: u64,
    /// Nanoseconds since the UNIX epoch at the moment the envelope was
    /// loaned for send. `REQ_0203`.
    pub timestamp_ns: u64,
    /// Application-controlled correlation id. The framework carries
    /// these bytes verbatim (`REQ_0204`); senders that do not need
    /// correlation should leave this zeroed.
    pub correlation_id: CorrelationId,
    /// Number of valid bytes in [`Self::payload`]. Always `<= N`.
    /// Receivers must trust this value (the framework validates it
    /// against `N` at send time — `TEST_0125`).
    pub payload_len: u32,
    /// Caller-defined metadata slot. Defaults to zero, and legacy
    /// senders (`send_raw_bytes`, [`Default`]) always write zero.
    /// Senders MAY stamp a non-zero value via `send_raw_bytes_v2` (or
    /// any future v2+ writer) to carry caller-defined metadata;
    /// receivers MUST NOT assume zero. The connector-zenoh layer is the
    /// only documented user today, where this field carries a per-call
    /// query timeout (`REQ_0425`).
    pub reserved: u32,
    /// Inline payload buffer. Only the first `payload_len` bytes are
    /// valid; the rest is uninitialised (the loan path may leave
    /// previously-sent bytes in the tail).
    pub payload: [u8; N],
}

impl<const N: usize> Default for ConnectorEnvelope<N> {
    fn default() -> Self {
        Self {
            sequence_number: 0,
            timestamp_ns: 0,
            correlation_id: [0u8; 32],
            payload_len: 0,
            reserved: 0,
            payload: [0u8; N],
        }
    }
}

impl<const N: usize> ConnectorEnvelope<N> {
    /// Maximum bytes the payload buffer can carry. Equals the `N` const
    /// generic; exposed as a function so callers don't have to remember
    /// the bound's name.
    #[must_use]
    pub const fn capacity() -> usize {
        N
    }

    /// Borrow the valid prefix of the payload buffer. The returned slice
    /// has length [`Self::payload_len`].
    #[must_use]
    pub fn payload_bytes(&self) -> &[u8] {
        let len = (self.payload_len as usize).min(N);
        &self.payload[..len]
    }
}
