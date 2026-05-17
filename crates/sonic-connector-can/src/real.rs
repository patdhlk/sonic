//! [`RealCanInterface`] — Linux-only `CanInterfaceLike` backed by
//! `socketcan::tokio::CanFdSocket`. Lives behind the
//! `socketcan-integration` cargo feature (`REQ_0503`) and is only
//! compiled on `cfg(target_os = "linux")` (`REQ_0502`).
//!
//! The kernel's `PF_CAN` raw socket family is Linux-specific; the
//! `socketcan` crate's build script rejects non-Linux targets, so
//! this module is target-gated rather than just feature-gated.
//!
//! ## Design choices
//!
//! * **Always FD-aware.** Every owned interface opens as
//!   [`socketcan::tokio::CanFdSocket`]. The kernel's FD-aware socket
//!   transparently accepts and emits classical frames as well
//!   (`CAN_RAW_FD_FRAMES` semantics — opened automatically by
//!   `CanFdSocket::open_addr`). This means one socket per interface
//!   regardless of whether the registered channels are classical,
//!   FD, or a mix — simpler resource model.
//!
//! * **Error frames enabled at open.** `set_error_filter_accept_all`
//!   is called immediately after `open` so the gateway's dispatcher
//!   sees bus-off / error-passive transitions via `CAN_ERR_FLAG`
//!   frames (`REQ_0631`).
//!
//! * **Reopen via close + re-open.** [`Self::reopen`] drops the old
//!   socket and constructs a fresh `CanFdSocket` for the same
//!   interface; this is the cleanest way to recover from bus-off
//!   when the kernel does not have `can-restart-ms` configured
//!   (`REQ_0533`, `REQ_0544`).

use socketcan::tokio::CanFdSocket;
use socketcan::{
    CanAnyFrame, CanDataFrame, CanError, CanErrorFrame, CanFdFrame, CanFilter as ScCanFilter,
    EmbeddedFrame, ExtendedId, Id, SocketOptions, StandardId, errors::ControllerProblem,
    frame::FdFlags as ScFdFlags,
};

use crate::driver::{
    CanData, CanErrorKind, CanFilter, CanFrame, CanIfaceState, CanInterfaceLike, CanIoError,
    MAX_CAN_PAYLOAD,
};
use crate::routing::{CanFdFlags, CanFrameKind, CanId, CanIface};

/// Linux-only real CAN interface backed by `socketcan::tokio::CanFdSocket`.
#[derive(Debug)]
pub struct RealCanInterface {
    iface: CanIface,
    state: CanIfaceState,
    /// `Some(socket)` while the interface is active. `None` after
    /// [`Self::reopen`] has dropped the prior socket but before the
    /// new one is constructed — never observed by external code
    /// because `reopen` either ends with `Some` (success) or returns
    /// `Err` (failure left for the caller to handle).
    socket: Option<CanFdSocket>,
}

impl RealCanInterface {
    /// Open one CAN interface by name (`vcan0`, `can0`, etc.). Sets
    /// the error-filter mask to accept all error frames so the
    /// gateway's classifier can drive `ConnectorHealth` transitions
    /// (`REQ_0631`).
    ///
    /// # Errors
    ///
    /// [`CanIoError::Io`] on socket open, error-filter setsockopt, or
    /// non-blocking flag setup failure.
    pub fn open(iface: CanIface) -> Result<Self, CanIoError> {
        let socket = CanFdSocket::open(iface.as_str())
            .map_err(|e| CanIoError::Io(format!("CanFdSocket::open({iface}): {e}")))?;
        socket
            .set_error_filter_accept_all()
            .map_err(|e| CanIoError::Io(format!("set_error_filter_accept_all: {e}")))?;
        Ok(Self {
            iface,
            state: CanIfaceState::Active,
            socket: Some(socket),
        })
    }

    fn require_socket(&mut self) -> Result<&mut CanFdSocket, CanIoError> {
        self.socket.as_mut().ok_or(CanIoError::Closed)
    }
}

/// Convert one of our `CanFilter` entries (with the CAN_EFF_FLAG
/// bit already folded into `can_id` / `can_mask` by
/// [`crate::filter::filter_for`]) into the upstream `socketcan`
/// representation. The kernel's `struct can_filter` semantics are
/// identical, so this is a transparent newtype shuffle.
fn to_socketcan_filter(f: &CanFilter) -> ScCanFilter {
    ScCanFilter::new(f.can_id, f.can_mask)
}

/// Convert our `CanId` into an `embedded_can::Id`. Unwraps are safe
/// because [`CanId::standard`] / [`CanId::extended`] already
/// bounds-check the raw values at construction.
fn to_id(id: CanId) -> Id {
    if id.extended {
        Id::Extended(ExtendedId::new(id.value).expect("CanId::extended bounded by construction"))
    } else {
        let v: u16 = u16::try_from(id.value).expect("standard CanId fits in 11 bits");
        Id::Standard(StandardId::new(v).expect("CanId::standard bounded by construction"))
    }
}

/// Convert an upstream classical frame's identifier back to our
/// `CanId`, preserving the extended discriminant.
fn from_id(id: Id) -> CanId {
    match id {
        Id::Standard(s) => CanId {
            value: u32::from(s.as_raw()),
            extended: false,
        },
        Id::Extended(e) => CanId {
            value: e.as_raw(),
            extended: true,
        },
    }
}

fn fd_flags_to_socketcan(flags: CanFdFlags) -> ScFdFlags {
    let mut out = ScFdFlags::empty();
    if flags.contains(CanFdFlags::BRS) {
        out |= ScFdFlags::BRS;
    }
    if flags.contains(CanFdFlags::ESI) {
        out |= ScFdFlags::ESI;
    }
    out
}

fn fd_flags_from_socketcan(flags: ScFdFlags) -> CanFdFlags {
    let mut out = CanFdFlags::empty();
    if flags.contains(ScFdFlags::BRS) {
        out |= CanFdFlags::BRS;
    }
    if flags.contains(ScFdFlags::ESI) {
        out |= CanFdFlags::ESI;
    }
    out
}

/// Translate an upstream classical data frame into our `CanData`.
fn data_from_classical(frame: &CanDataFrame) -> CanData {
    let bytes = frame.data();
    let mut buf = [0u8; MAX_CAN_PAYLOAD];
    let n = bytes.len().min(buf.len());
    buf[..n].copy_from_slice(&bytes[..n]);
    CanData {
        id: from_id(frame.id()),
        kind: CanFrameKind::Classical,
        fd_flags: CanFdFlags::empty(),
        len: n as u8,
        bytes: buf,
    }
}

/// Translate an upstream FD frame into our `CanData`.
fn data_from_fd(frame: &CanFdFrame) -> CanData {
    let bytes = frame.data();
    let mut buf = [0u8; MAX_CAN_PAYLOAD];
    let n = bytes.len().min(buf.len());
    buf[..n].copy_from_slice(&bytes[..n]);
    CanData {
        id: from_id(frame.id()),
        kind: CanFrameKind::Fd,
        fd_flags: fd_flags_from_socketcan(frame.flags()),
        len: n as u8,
        bytes: buf,
    }
}

/// Classify an upstream error frame into our `CanErrorKind`.
fn classify_error(frame: CanErrorFrame) -> CanErrorKind {
    match frame.into_error() {
        CanError::BusOff => CanErrorKind::BusOff,
        CanError::ControllerProblem(p) => {
            match p {
                ControllerProblem::ReceiveErrorPassive
                | ControllerProblem::TransmitErrorPassive => CanErrorKind::Passive,
                ControllerProblem::ReceiveErrorWarning
                | ControllerProblem::TransmitErrorWarning => CanErrorKind::Warning,
                _ => CanErrorKind::Other,
            }
        }
        CanError::LostArbitration(_) => CanErrorKind::ArbitrationLost,
        // Treat bus errors, no-ACK, transceiver errors, protocol
        // violations, restarts, and undecodable variants as Other —
        // they are surface-level diagnostics that do not by themselves
        // require a health transition under our `ARCH_0062` machine.
        _ => CanErrorKind::Other,
    }
}

/// Build an upstream classical [`CanDataFrame`] from our [`CanData`].
///
/// # Errors
///
/// [`CanIoError::Io`] when the upstream constructor rejects the
/// (id, data) pair — typically because `data.len() > 8`.
fn build_classical(data: &CanData) -> Result<CanDataFrame, CanIoError> {
    let id = to_id(data.id);
    CanDataFrame::new(id, data.payload()).ok_or_else(|| {
        CanIoError::Io(format!(
            "CanDataFrame::new rejected payload len {}",
            data.len
        ))
    })
}

/// Build an upstream FD frame from our [`CanData`].
fn build_fd(data: &CanData) -> Result<CanFdFrame, CanIoError> {
    let id = to_id(data.id);
    let flags = fd_flags_to_socketcan(data.fd_flags);
    CanFdFrame::with_flags(id, data.payload(), flags).ok_or_else(|| {
        CanIoError::Io(format!(
            "CanFdFrame::with_flags rejected payload len {}",
            data.len
        ))
    })
}

impl CanInterfaceLike for RealCanInterface {
    fn iface(&self) -> &CanIface {
        &self.iface
    }

    fn state(&self) -> CanIfaceState {
        self.state
    }

    fn apply_filter(&mut self, filters: &[CanFilter]) -> Result<(), CanIoError> {
        let socket = self.require_socket()?;
        let sc_filters: Vec<ScCanFilter> = filters.iter().map(to_socketcan_filter).collect();
        socket
            .set_filters(&sc_filters)
            .map_err(|e| CanIoError::Io(format!("set_filters: {e}")))
    }

    fn recv(
        &mut self,
    ) -> impl core::future::Future<Output = Result<CanFrame, CanIoError>> + Send + '_ {
        async move {
            let socket = self.require_socket()?;
            let any = socket
                .read_frame()
                .await
                .map_err(|e| CanIoError::Io(format!("read_frame: {e}")))?;
            Ok(map_any_frame(any))
        }
    }

    fn send_classical(
        &mut self,
        frame: &CanData,
    ) -> impl core::future::Future<Output = Result<(), CanIoError>> + Send + '_ {
        let frame = *frame;
        async move {
            let sc_frame = build_classical(&frame)?;
            let any = CanAnyFrame::from(sc_frame);
            let socket = self.require_socket()?;
            socket
                .write_frame(&any)
                .await
                .map_err(|e| CanIoError::Io(format!("write_frame classical: {e}")))
        }
    }

    fn send_fd(
        &mut self,
        frame: &CanData,
    ) -> impl core::future::Future<Output = Result<(), CanIoError>> + Send + '_ {
        let frame = *frame;
        async move {
            let sc_frame = build_fd(&frame)?;
            let any = CanAnyFrame::Fd(sc_frame);
            let socket = self.require_socket()?;
            socket
                .write_frame(&any)
                .await
                .map_err(|e| CanIoError::Io(format!("write_frame fd: {e}")))
        }
    }

    fn reopen(&mut self) -> impl core::future::Future<Output = Result<(), CanIoError>> + Send + '_ {
        async move {
            // Drop the prior socket first so the kernel sees a fresh
            // bind on reopen.
            self.socket = None;
            self.state = CanIfaceState::Closed;
            let socket = CanFdSocket::open(self.iface.as_str())
                .map_err(|e| CanIoError::Io(format!("reopen CanFdSocket::open: {e}")))?;
            socket
                .set_error_filter_accept_all()
                .map_err(|e| CanIoError::Io(format!("reopen set_error_filter: {e}")))?;
            self.socket = Some(socket);
            self.state = CanIfaceState::Active;
            Ok(())
        }
    }
}

fn map_any_frame(any: CanAnyFrame) -> CanFrame {
    match any {
        CanAnyFrame::Normal(d) => CanFrame::Data(data_from_classical(&d)),
        CanAnyFrame::Remote(r) => CanFrame::Remote {
            id: from_id(r.id()),
            dlc: r.dlc() as u8,
        },
        CanAnyFrame::Error(e) => CanFrame::Error(classify_error(e)),
        CanAnyFrame::Fd(fd) => CanFrame::Data(data_from_fd(&fd)),
    }
}

#[cfg(test)]
mod tests {
    //! Pure-logic tests for the type conversions. Real socket I/O is
    //! exercised by the layer-2 `tests/vcan_smoke.rs` integration test
    //! which requires the `vcan` kernel module.

    use super::*;

    #[test]
    fn id_round_trip_standard() {
        let original = CanId::standard(0x123).unwrap();
        let id = to_id(original);
        assert_eq!(from_id(id), original);
    }

    #[test]
    fn id_round_trip_extended() {
        let original = CanId::extended(0x1ABCDEF).unwrap();
        let id = to_id(original);
        assert_eq!(from_id(id), original);
    }

    #[test]
    fn fd_flags_round_trip() {
        let original = CanFdFlags::BRS | CanFdFlags::ESI;
        let sc = fd_flags_to_socketcan(original);
        assert_eq!(fd_flags_from_socketcan(sc), original);
    }

    #[test]
    fn filter_conversion_preserves_eff_bits() {
        use crate::driver::CAN_EFF_FLAG;
        let f = CanFilter {
            can_id: 0x123 | CAN_EFF_FLAG,
            can_mask: 0x7FF | CAN_EFF_FLAG,
        };
        let sc = to_socketcan_filter(&f);
        // ScCanFilter does not expose its inner fields, but the
        // round-trip via its Debug impl preserves the bits.
        let dbg = format!("{sc:?}");
        assert!(
            dbg.contains("8000_07FF")
                || dbg.contains("80000123")
                || dbg.contains(&format!("{:08x}", f.can_id)),
            "filter debug should contain folded can_id; got: {dbg}"
        );
    }

    #[test]
    fn classify_error_busoff() {
        // CanErrorFrame's error_bits == 0x40 maps to CanError::BusOff
        // → our CanErrorKind::BusOff.
        let frame = CanErrorFrame::new_error(0x0040, &[0; 8]).unwrap();
        assert_eq!(classify_error(frame), CanErrorKind::BusOff);
    }

    #[test]
    fn classify_error_passive() {
        // error_bits 0x04 + data[1] = 0x10 (RX passive) -> Passive.
        let mut data = [0u8; 8];
        data[1] = 0x10;
        let frame = CanErrorFrame::new_error(0x0004, &data).unwrap();
        assert_eq!(classify_error(frame), CanErrorKind::Passive);
    }

    #[test]
    fn classify_error_warning() {
        let mut data = [0u8; 8];
        data[1] = 0x04;
        let frame = CanErrorFrame::new_error(0x0004, &data).unwrap();
        assert_eq!(classify_error(frame), CanErrorKind::Warning);
    }

    #[test]
    fn build_classical_8_byte_frame_succeeds() {
        let data = CanData::new(
            CanId::standard(0x100).unwrap(),
            CanFrameKind::Classical,
            CanFdFlags::empty(),
            &[1, 2, 3, 4, 5, 6, 7, 8],
        )
        .unwrap();
        let frame = build_classical(&data).expect("8-byte classical frame builds");
        assert_eq!(frame.data().len(), 8);
        assert_eq!(frame.id(), to_id(CanId::standard(0x100).unwrap()));
    }
}
