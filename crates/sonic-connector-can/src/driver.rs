//! [`CanInterfaceLike`] — async trait every CAN back-end implements.
//! `BB_0072`.
//!
//! Two known implementations:
//!
//! * [`crate::MockCanInterface`] — in-process loopback with
//!   programmable error injection. Used by all layer-1 tests.
//! * `RealCanInterface` — wraps `socketcan::tokio::CanSocket` /
//!   `CanFdSocket`. Lives behind the `socketcan-integration` cargo
//!   feature and lands in a follow-on commit (layer-2).
//!
//! Frame shape is allocation-free on the hot path: [`CanData`] carries
//! a fixed `[u8; 64]` buffer plus length, sized to the FD maximum.
//! Classical frames use the first 0–8 bytes.

use crate::routing::{CanFdFlags, CanFrameKind, CanId, CanIface};

/// Maximum payload length (CAN-FD upper bound, in bytes).
pub const MAX_CAN_PAYLOAD: usize = 64;

/// Inline-buffered CAN data frame. `Clone` is cheap (~80 bytes).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CanData {
    /// CAN identifier (standard / extended distinguished by the
    /// `extended` flag).
    pub id: CanId,
    /// Classical or FD; determines max length.
    pub kind: CanFrameKind,
    /// FD flags (BRS / ESI). Always [`CanFdFlags::empty`] when
    /// `kind == Classical`.
    pub fd_flags: CanFdFlags,
    /// Number of valid bytes in [`Self::bytes`].
    pub len: u8,
    /// Inline payload buffer. Indices `[len..]` are unspecified.
    pub bytes: [u8; MAX_CAN_PAYLOAD],
}

impl CanData {
    /// Construct from `id`, `kind`, and a payload slice. The slice is
    /// copied into the inline buffer.
    ///
    /// # Errors
    ///
    /// Returns [`CanIoError::PayloadTooLong`] when the slice exceeds
    /// `kind.max_payload()`.
    pub fn new(
        id: CanId,
        kind: CanFrameKind,
        fd_flags: CanFdFlags,
        payload: &[u8],
    ) -> Result<Self, CanIoError> {
        let max = kind.max_payload();
        if payload.len() > max {
            return Err(CanIoError::PayloadTooLong {
                actual: payload.len(),
                max,
            });
        }
        let mut bytes = [0u8; MAX_CAN_PAYLOAD];
        bytes[..payload.len()].copy_from_slice(payload);
        Ok(Self {
            id,
            kind,
            fd_flags: if matches!(kind, CanFrameKind::Classical) {
                CanFdFlags::empty()
            } else {
                fd_flags
            },
            len: payload.len() as u8,
            bytes,
        })
    }

    /// Borrow the valid payload bytes.
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
}

/// Classified CAN error condition derived from a kernel error frame
/// (`CAN_ERR_FLAG`) or an equivalent mock-injected event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CanErrorKind {
    /// Error counter crossed the warning threshold (typically 96).
    Warning,
    /// Controller entered error-passive state (`REQ_0632`).
    Passive,
    /// Controller entered bus-off state (`REQ_0633`).
    BusOff,
    /// Arbitration lost — informational, does not change health.
    ArbitrationLost,
    /// Other / unclassified error frame.
    Other,
}

/// Frame yielded by [`CanInterfaceLike::recv`]. Three discriminants
/// mirror the upstream `socketcan::CanFrame` shape so layer-2 can be
/// a thin wrapper.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CanFrame {
    /// Data frame — the common case.
    Data(CanData),
    /// Remote-transmission request (RTR). Layer-1 surfaces these but
    /// does not act on them; layer-2 may reject when configured.
    Remote {
        /// Identifier of the requested object.
        id: CanId,
        /// Requested data-length code (DLC).
        dlc: u8,
    },
    /// Classified error frame. Consumed by the gateway's error
    /// classifier (`REQ_0531`); never delivered to a plugin channel
    /// (`REQ_0536`).
    Error(CanErrorKind),
}

/// One CAN filter entry. Matches the `linux/can.h` `struct can_filter`
/// layout (`can_id` + `can_mask`); the `extended` flag is folded into
/// `can_id`'s high-bit per the kernel's `CAN_EFF_FLAG` convention so
/// the match predicate is a single `&` per frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct CanFilter {
    /// Identifier to match, with the `CAN_EFF_FLAG` (`1 << 31`) bit
    /// set when matching extended IDs.
    pub can_id: u32,
    /// Mask. A frame matches when `(frame_id ^ can_id) & can_mask == 0`.
    pub can_mask: u32,
}

/// Bit position of `CAN_EFF_FLAG` in the folded filter id.
pub const CAN_EFF_FLAG: u32 = 0x8000_0000;

/// Bus-state observed by the gateway. Maps onto
/// [`crate::IfaceHealthKind`] via the dispatcher's classifier
/// (`ARCH_0062`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CanIfaceState {
    /// Socket is not bound (initial state, or post-bus-off close).
    #[default]
    Closed,
    /// Bound and operational.
    Active,
    /// Bus-off — needs reopen.
    BusOff,
}

/// I/O failure modes shared between mock and real interfaces.
#[derive(Debug, thiserror::Error)]
pub enum CanIoError {
    /// Caller tried to send a payload exceeding the kind's max.
    #[error("CAN payload too long: {actual} > {max}")]
    PayloadTooLong {
        /// Bytes the caller provided.
        actual: usize,
        /// Maximum allowed for the frame kind.
        max: usize,
    },
    /// Underlying socket / kernel returned an error.
    #[error("CAN I/O: {0}")]
    Io(String),
    /// Interface is closed / not opened.
    #[error("CAN interface is closed")]
    Closed,
    /// Interface is in bus-off state.
    #[error("CAN interface is in bus-off")]
    BusOff,
}

/// Driver-side contract every CAN back-end implements.
///
/// `Send + 'static` because the dispatcher owns each driver by value
/// on a tokio task.
pub trait CanInterfaceLike: Send + 'static {
    /// Borrow the interface this back-end is bound to.
    fn iface(&self) -> &CanIface;

    /// Snapshot the current bus state. Cheap (no I/O).
    fn state(&self) -> CanIfaceState;

    /// Apply the per-interface filter set. The kernel filters out
    /// non-matching frames before they reach the read loop
    /// (`REQ_0522`).
    ///
    /// # Errors
    ///
    /// [`CanIoError::Io`] on `setsockopt` failure.
    fn apply_filter(&mut self, filters: &[CanFilter]) -> Result<(), CanIoError>;

    /// Receive the next frame (data, remote, or error). Awaits until
    /// one is available or the socket closes.
    fn recv(
        &mut self,
    ) -> impl core::future::Future<Output = Result<CanFrame, CanIoError>> + Send + '_;

    /// Send a classical (≤ 8 bytes) frame.
    fn send_classical(
        &mut self,
        frame: &CanData,
    ) -> impl core::future::Future<Output = Result<(), CanIoError>> + Send + '_;

    /// Send a CAN-FD (≤ 64 bytes) frame.
    fn send_fd(
        &mut self,
        frame: &CanData,
    ) -> impl core::future::Future<Output = Result<(), CanIoError>> + Send + '_;

    /// Reopen the socket after a bus-off close. Dispatcher invokes
    /// this on the configured [`sonic_connector_core::ReconnectPolicy`]
    /// backoff (`REQ_0533`, `REQ_0534`).
    fn reopen(&mut self) -> impl core::future::Future<Output = Result<(), CanIoError>> + Send + '_;
}
