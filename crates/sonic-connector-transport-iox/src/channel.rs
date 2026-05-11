//! [`ChannelWriter`] and [`ChannelReader`] — typed publisher / subscriber
//! handles over a single iceoryx2 pub/sub service.
//!
//! These types are returned by [`crate::ServiceFactory`] and form the
//! plugin / gateway-facing API for sending and receiving typed payloads.

use core::marker::PhantomData;
use core::sync::atomic::{AtomicU64, Ordering};

use iceoryx2::port::publisher::Publisher;
use iceoryx2::port::subscriber::Subscriber;
use iceoryx2::prelude::ipc;
use sonic_connector_core::{ConnectorError, PayloadCodec};

use crate::envelope::{ConnectorEnvelope, CorrelationId};
use crate::now::now_unix_ns;

/// Typed publisher handle. Owns an iceoryx2 [`Publisher`] over
/// [`ConnectorEnvelope<N>`], an instance of the connector's codec, and
/// a per-handle monotonically increasing sequence counter (`REQ_0202`).
///
/// `T` is the application payload type. `C` is the codec. `N` is the
/// channel's maximum payload size in bytes.
pub struct ChannelWriter<T, C, const N: usize> {
    inner: Publisher<ipc::Service, ConnectorEnvelope<N>, ()>,
    codec: C,
    sequence: AtomicU64,
    _phantom: PhantomData<fn(T)>,
}

// SAFETY: iceoryx2's `Publisher<ipc::Service, …>` is conditionally
// `Send` (see `sonic-executor`'s wrapper for the equivalent rationale).
// After construction, the only per-iteration call is `loan_uninit` /
// `send`, which do not concurrently mutate the inner Rc.
#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl<T: Send, C: Send, const N: usize> Send for ChannelWriter<T, C, N> {}

impl<T, C, const N: usize> ChannelWriter<T, C, N>
where
    C: PayloadCodec,
    T: serde::Serialize,
{
    pub(crate) const fn new(
        inner: Publisher<ipc::Service, ConnectorEnvelope<N>, ()>,
        codec: C,
    ) -> Self {
        Self {
            inner,
            codec,
            sequence: AtomicU64::new(0),
            _phantom: PhantomData,
        }
    }

    /// Send `value` with a zeroed correlation id.
    pub fn send(&self, value: &T) -> Result<SendOutcome, ConnectorError> {
        self.send_with_correlation(value, [0u8; 32])
    }

    /// Send `value` with the caller-supplied correlation id. `REQ_0204`.
    pub fn send_with_correlation(
        &self,
        value: &T,
        correlation_id: CorrelationId,
    ) -> Result<SendOutcome, ConnectorError> {
        let mut sample = self
            .inner
            .loan_uninit()
            .map_err(|e| ConnectorError::stack(IoxLoanError(format!("{e:?}"))))?;

        // Encode FIRST, before consuming a sequence number. A codec
        // failure drops the loan (no envelope on the wire) and leaves
        // the sequence counter untouched — exercised by TEST_0125.
        let slot = sample.payload_mut();
        let env_ptr = slot.as_mut_ptr();
        let payload_array_ptr: *mut [u8; N] =
            unsafe { core::ptr::addr_of_mut!((*env_ptr).payload) };
        // SAFETY: `payload_array_ptr` points at the inline `[u8; N]`
        // field inside the iceoryx2-owned SHM slot; the slot is valid
        // for writes for the lifetime of `sample`. We treat the array
        // as `N` initialised bytes (`u8` has no validity invariants) so
        // a `&mut [u8]` is sound regardless of prior content.
        let payload_buf: &mut [u8] =
            unsafe { core::slice::from_raw_parts_mut(payload_array_ptr.cast::<u8>(), N) };

        let written = self.codec.encode(value, payload_buf)?;
        if written > N {
            // Defensive: codec returned a written-byte count exceeding
            // the buffer. Treat as overflow and drop the loan.
            return Err(ConnectorError::PayloadOverflow {
                actual: written,
                max: N,
            });
        }
        let written_u32 = u32::try_from(written).map_err(|_| ConnectorError::PayloadOverflow {
            actual: written,
            max: N,
        })?;

        // Encode succeeded — claim the next sequence number and stamp
        // the header.
        let seq = self.sequence.fetch_add(1, Ordering::Relaxed);
        let ts = now_unix_ns();

        // SAFETY: write each remaining header field through the slot's
        // raw pointer. After this block every byte of the envelope
        // (header + payload bytes 0..written + uninitialised tail) is
        // accessible. The trailing `payload[written..N]` bytes are
        // technically uninitialised, but `u8` has no validity invariants,
        // so the slot satisfies `assume_init`. Receivers read only the
        // first `payload_len` bytes (`payload_bytes`), so the tail is
        // never observed.
        unsafe {
            core::ptr::addr_of_mut!((*env_ptr).sequence_number).write(seq);
            core::ptr::addr_of_mut!((*env_ptr).timestamp_ns).write(ts);
            core::ptr::addr_of_mut!((*env_ptr).correlation_id).write(correlation_id);
            core::ptr::addr_of_mut!((*env_ptr).payload_len).write(written_u32);
            core::ptr::addr_of_mut!((*env_ptr).reserved).write(0);
        }

        // SAFETY: every field of `ConnectorEnvelope` is now initialised
        // per the comment above. `payload` past `written` is `u8` so
        // uninit reads would be UB only if observed — receivers honour
        // `payload_len`.
        let sample = unsafe { sample.assume_init() };
        sample
            .send()
            .map_err(|e| ConnectorError::stack(IoxLoanError(format!("{e:?}"))))?;

        Ok(SendOutcome {
            sequence_number: seq,
            timestamp_ns: ts,
            bytes_written: written,
        })
    }
}

/// Outcome of a successful [`ChannelWriter::send`] call. Useful in
/// tests and metrics to inspect the wire-level state without re-reading
/// the envelope.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct SendOutcome {
    /// The sequence number stamped into the envelope.
    pub sequence_number: u64,
    /// The timestamp stamped into the envelope.
    pub timestamp_ns: u64,
    /// Number of payload bytes the codec wrote.
    pub bytes_written: usize,
}

/// Typed subscriber handle. Mirrors [`ChannelWriter`].
pub struct ChannelReader<T, C, const N: usize> {
    inner: Subscriber<ipc::Service, ConnectorEnvelope<N>, ()>,
    codec: C,
    _phantom: PhantomData<fn() -> T>,
}

#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl<T: Send, C: Send, const N: usize> Send for ChannelReader<T, C, N> {}

impl<T, C, const N: usize> ChannelReader<T, C, N>
where
    C: PayloadCodec,
    T: serde::de::DeserializeOwned,
{
    pub(crate) const fn new(
        inner: Subscriber<ipc::Service, ConnectorEnvelope<N>, ()>,
        codec: C,
    ) -> Self {
        Self {
            inner,
            codec,
            _phantom: PhantomData,
        }
    }

    /// Take the next envelope, if any, and decode its payload into `T`.
    ///
    /// Returns `Ok(None)` when no envelope is available. Decode errors
    /// surface as [`ConnectorError::Codec`] rather than silently
    /// dropping the envelope (`REQ_0214`).
    pub fn try_recv(&self) -> Result<Option<RecvEnvelope<T>>, ConnectorError> {
        let Some(sample) = self
            .inner
            .receive()
            .map_err(|e| ConnectorError::stack(IoxLoanError(format!("{e:?}"))))?
        else {
            return Ok(None);
        };
        let env: &ConnectorEnvelope<N> = sample.payload();
        let value = self.codec.decode(env.payload_bytes())?;
        Ok(Some(RecvEnvelope {
            sequence_number: env.sequence_number,
            timestamp_ns: env.timestamp_ns,
            correlation_id: env.correlation_id,
            value,
        }))
    }
}

/// Decoded envelope handed back from [`ChannelReader::try_recv`].
#[derive(Clone, Debug)]
pub struct RecvEnvelope<T> {
    /// Sequence number stamped by the sender.
    pub sequence_number: u64,
    /// Sender timestamp (UNIX nanoseconds).
    pub timestamp_ns: u64,
    /// Correlation id carried verbatim from sender to receiver
    /// (`REQ_0204`).
    pub correlation_id: CorrelationId,
    /// Decoded application payload.
    pub value: T,
}

/// Adapter that converts an iceoryx2 error string into a
/// [`std::error::Error`] for [`ConnectorError::stack`]. Internal — the
/// public-facing error variant is [`ConnectorError::Stack`].
#[derive(Debug)]
struct IoxLoanError(String);

impl core::fmt::Display for IoxLoanError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "iceoryx2: {}", self.0)
    }
}

impl std::error::Error for IoxLoanError {}
