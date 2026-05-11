//! Byte-only iceoryx2 channel handles for the gateway dispatcher.
//! `REQ_0326`, `REQ_0327`.
//!
//! [`RawChannelWriter`] and [`RawChannelReader`] mirror
//! [`crate::ChannelWriter`] / [`crate::ChannelReader`] but elide both
//! the user payload type `T` and the [`PayloadCodec`] generic — the
//! gateway-side of the `EtherCAT` connector pipeline moves bytes
//! verbatim between iceoryx2 services and the `SubDevice` PDI, so the
//! codec never runs on this path. Symmetric pairing with the plugin's
//! typed handles: a `ChannelWriter<T, C, N>` and a
//! `RawChannelReader<N>` opened against the same iceoryx2 service
//! name see the same envelopes; only the reader skips decoding.
//!
//! The `try_recv_into` shape (caller-supplied destination buffer)
//! preserves the gateway dispatcher's steady-state allocation-free
//! invariant: no `Vec` is materialised per envelope. The dispatcher
//! provides a stack-allocated scratch buffer sized to the channel's
//! `N` const generic.
//!
//! [`PayloadCodec`]: sonic_connector_core::PayloadCodec

use core::sync::atomic::{AtomicU64, Ordering};

use iceoryx2::port::publisher::Publisher;
use iceoryx2::port::subscriber::Subscriber;
use iceoryx2::prelude::ipc;
use sonic_connector_core::ConnectorError;

use crate::envelope::{ConnectorEnvelope, CorrelationId};
use crate::now::now_unix_ns;

/// Outcome of a successful [`RawChannelWriter::send_raw_bytes`] call.
///
/// Mirrors [`crate::channel::SendOutcome`] for symmetry; kept separate
/// so the raw API can evolve independently if zero-copy publishing
/// ever lands.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct RawSendOutcome {
    /// Sequence number stamped into the envelope (`REQ_0202`).
    pub sequence_number: u64,
    /// Timestamp stamped into the envelope (`REQ_0203`).
    pub timestamp_ns: u64,
    /// Number of payload bytes actually copied into the envelope.
    pub bytes_written: usize,
}

/// Metadata returned alongside a `try_recv_into` call.
///
/// The payload itself is written into the caller's destination
/// buffer; this struct carries only the header fields plus the actual
/// length the caller should slice to.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct RawSample {
    /// Sequence number stamped by the sender (`REQ_0202`).
    pub sequence_number: u64,
    /// Sender timestamp (`REQ_0203`).
    pub timestamp_ns: u64,
    /// Correlation id carried verbatim (`REQ_0204`).
    pub correlation_id: CorrelationId,
    /// Number of payload bytes written into the caller-supplied
    /// buffer. Always `<= caller_buffer.len()` and `<= N`.
    pub payload_len: usize,
}

/// Byte-only iceoryx2 publisher. Owns a [`Publisher`] over
/// [`ConnectorEnvelope<N>`] and a per-handle monotonic sequence
/// counter; no codec, no payload type.
pub struct RawChannelWriter<const N: usize> {
    inner: Publisher<ipc::Service, ConnectorEnvelope<N>, ()>,
    sequence: AtomicU64,
}

// SAFETY: same rationale as `ChannelWriter` — iceoryx2 publishers are
// conditionally `Send` and the only per-call API is `loan_uninit` /
// `send`, neither of which races with itself.
#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl<const N: usize> Send for RawChannelWriter<N> {}

impl<const N: usize> RawChannelWriter<N> {
    pub(crate) const fn new(inner: Publisher<ipc::Service, ConnectorEnvelope<N>, ()>) -> Self {
        Self {
            inner,
            sequence: AtomicU64::new(0),
        }
    }

    /// Publish `payload` verbatim. No codec invocation.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectorError::PayloadOverflow`] when `payload.len()`
    /// exceeds the channel's compile-time capacity `N`. Returns
    /// [`ConnectorError::Stack`] wrapping any iceoryx2 loan / send
    /// error.
    pub fn send_raw_bytes(
        &self,
        payload: &[u8],
        correlation_id: CorrelationId,
    ) -> Result<RawSendOutcome, ConnectorError> {
        let written = payload.len();
        if written > N {
            return Err(ConnectorError::PayloadOverflow {
                actual: written,
                max: N,
            });
        }
        let written_u32 = u32::try_from(written).map_err(|_| ConnectorError::PayloadOverflow {
            actual: written,
            max: N,
        })?;

        let mut sample = self
            .inner
            .loan_uninit()
            .map_err(|e| ConnectorError::stack(RawError(format!("loan: {e:?}"))))?;

        let slot = sample.payload_mut();
        let env_ptr = slot.as_mut_ptr();

        // SAFETY: see ChannelWriter::send_with_correlation. `slot` is
        // a valid iceoryx2-owned SHM region; field-by-field raw writes
        // cover every byte of the envelope before `assume_init`.
        let payload_array_ptr: *mut [u8; N] =
            unsafe { core::ptr::addr_of_mut!((*env_ptr).payload) };
        let payload_buf: &mut [u8] =
            unsafe { core::slice::from_raw_parts_mut(payload_array_ptr.cast::<u8>(), N) };
        payload_buf[..written].copy_from_slice(payload);
        // The tail [written..N] is left uninitialised; `u8` has no
        // validity invariants and receivers read only the first
        // `payload_len` bytes.

        let seq = self.sequence.fetch_add(1, Ordering::Relaxed);
        let ts = now_unix_ns();
        // SAFETY: every header field is written through the raw
        // pointer below; after this block the entire envelope is
        // initialised (payload tail is uninit `u8`, which is fine).
        unsafe {
            core::ptr::addr_of_mut!((*env_ptr).sequence_number).write(seq);
            core::ptr::addr_of_mut!((*env_ptr).timestamp_ns).write(ts);
            core::ptr::addr_of_mut!((*env_ptr).correlation_id).write(correlation_id);
            core::ptr::addr_of_mut!((*env_ptr).payload_len).write(written_u32);
            core::ptr::addr_of_mut!((*env_ptr).reserved).write(0);
        }
        // SAFETY: every field initialised per the comment above.
        let sample = unsafe { sample.assume_init() };
        sample
            .send()
            .map_err(|e| ConnectorError::stack(RawError(format!("send: {e:?}"))))?;

        Ok(RawSendOutcome {
            sequence_number: seq,
            timestamp_ns: ts,
            bytes_written: written,
        })
    }
}

/// Byte-only iceoryx2 subscriber. Drains envelopes into a
/// caller-provided destination buffer; never allocates.
pub struct RawChannelReader<const N: usize> {
    inner: Subscriber<ipc::Service, ConnectorEnvelope<N>, ()>,
}

#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl<const N: usize> Send for RawChannelReader<N> {}

impl<const N: usize> RawChannelReader<N> {
    pub(crate) const fn new(inner: Subscriber<ipc::Service, ConnectorEnvelope<N>, ()>) -> Self {
        Self { inner }
    }

    /// Drain one envelope into `dest`. Returns `Ok(Some(sample))`
    /// when an envelope was consumed and its payload copied into
    /// `dest[..sample.payload_len]`; returns `Ok(None)` when the
    /// subscriber's queue was empty.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectorError::PayloadOverflow`] when the envelope's
    /// `payload_len` exceeds `dest.len()`. Returns
    /// [`ConnectorError::Stack`] wrapping any iceoryx2 receive error.
    pub fn try_recv_into(&self, dest: &mut [u8]) -> Result<Option<RawSample>, ConnectorError> {
        let Some(sample) = self
            .inner
            .receive()
            .map_err(|e| ConnectorError::stack(RawError(format!("receive: {e:?}"))))?
        else {
            return Ok(None);
        };
        let env: &ConnectorEnvelope<N> = sample.payload();
        let bytes = env.payload_bytes();
        if bytes.len() > dest.len() {
            return Err(ConnectorError::PayloadOverflow {
                actual: bytes.len(),
                max: dest.len(),
            });
        }
        dest[..bytes.len()].copy_from_slice(bytes);
        Ok(Some(RawSample {
            sequence_number: env.sequence_number,
            timestamp_ns: env.timestamp_ns,
            correlation_id: env.correlation_id,
            payload_len: bytes.len(),
        }))
    }
}

#[derive(Debug)]
struct RawError(String);

impl core::fmt::Display for RawError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "iceoryx2 raw: {}", self.0)
    }
}

impl std::error::Error for RawError {}
