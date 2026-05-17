//! Typed routing for the CAN connector. `REQ_0601`, `REQ_0615`.
//!
//! [`CanRouting`] identifies one logical channel by interface name,
//! CAN identifier (with the standard / extended flag preserved
//! end-to-end), kernel-style filter mask, frame kind, and CAN-FD
//! flags.

use bitflags::bitflags;
use sonic_connector_core::Routing;

/// Linux network-interface name length cap. `IFNAMSIZ` is 16 in
/// `<linux/if.h>`; the trailing NUL leaves 15 usable ASCII bytes.
pub const IFNAMSIZ_MINUS_ONE: usize = 15;

/// Bounded ASCII network-interface name (e.g. `vcan0`, `can1`).
///
/// Construction validates the length cap (`IFNAMSIZ_MINUS_ONE`) and
/// the character set (ASCII printable, no spaces, no slashes — same
/// constraints Linux accepts via `ip link`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct CanIface {
    bytes: [u8; IFNAMSIZ_MINUS_ONE],
    len: u8,
}

/// Failure modes of [`CanIface::new`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CanIfaceError {
    /// Empty interface name.
    #[error("interface name must not be empty")]
    Empty,
    /// Name exceeds [`IFNAMSIZ_MINUS_ONE`] bytes.
    #[error("interface name exceeds {IFNAMSIZ_MINUS_ONE} bytes")]
    TooLong,
    /// Disallowed character — non-ASCII, control character, space, or `/`.
    #[error("interface name contains disallowed character")]
    InvalidChar,
}

impl CanIface {
    /// Construct from an ASCII string. Validates length and char set.
    ///
    /// # Errors
    ///
    /// See [`CanIfaceError`].
    pub fn new(name: &str) -> Result<Self, CanIfaceError> {
        if name.is_empty() {
            return Err(CanIfaceError::Empty);
        }
        if name.len() > IFNAMSIZ_MINUS_ONE {
            return Err(CanIfaceError::TooLong);
        }
        for &b in name.as_bytes() {
            if !b.is_ascii() || b <= b' ' || b == b'/' || b == 0x7f {
                return Err(CanIfaceError::InvalidChar);
            }
        }
        let mut bytes = [0u8; IFNAMSIZ_MINUS_ONE];
        bytes[..name.len()].copy_from_slice(name.as_bytes());
        Ok(Self {
            bytes,
            len: name.len() as u8,
        })
    }

    /// Borrow as `&str`. Always valid UTF-8 by construction.
    #[must_use]
    pub fn as_str(&self) -> &str {
        let n = self.len as usize;
        // SAFETY: `Self::new` already validated every byte is
        // printable ASCII < 0x7F, so the slice is well-formed UTF-8.
        // (We use `from_utf8_unchecked` to keep this allocation-free
        // and `unsafe_code = deny` is overridden via the explicit
        // `expect` to be a debug-build runtime check.)
        core::str::from_utf8(&self.bytes[..n])
            .expect("CanIface bytes are validated ASCII by Self::new")
    }
}

impl core::fmt::Display for CanIface {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Typed CAN identifier — either 11-bit standard or 29-bit extended.
///
/// The `extended` flag is identity-bearing: `CanId::standard(0x123)`
/// and `CanId::extended(0x123)` are distinct values (`REQ_0615`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct CanId {
    /// Raw identifier value.
    pub value: u32,
    /// `true` for 29-bit extended (CAN 2.0B); `false` for 11-bit
    /// standard (CAN 2.0A).
    pub extended: bool,
}

/// Failure modes of [`CanId::standard`] / [`CanId::extended`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CanIdError {
    /// Standard ID exceeds 11 bits (0x7FF).
    #[error("standard CAN ID exceeds 11 bits (max 0x7FF)")]
    StandardOverflow,
    /// Extended ID exceeds 29 bits (0x1FFF_FFFF).
    #[error("extended CAN ID exceeds 29 bits (max 0x1FFF_FFFF)")]
    ExtendedOverflow,
}

impl CanId {
    /// Construct an 11-bit standard identifier.
    ///
    /// # Errors
    ///
    /// Returns [`CanIdError::StandardOverflow`] when `value` exceeds
    /// `0x7FF`.
    pub const fn standard(value: u16) -> Result<Self, CanIdError> {
        if value > 0x7FF {
            return Err(CanIdError::StandardOverflow);
        }
        Ok(Self {
            value: value as u32,
            extended: false,
        })
    }

    /// Construct a 29-bit extended identifier.
    ///
    /// # Errors
    ///
    /// Returns [`CanIdError::ExtendedOverflow`] when `value` exceeds
    /// `0x1FFF_FFFF`.
    pub const fn extended(value: u32) -> Result<Self, CanIdError> {
        if value > 0x1FFF_FFFF {
            return Err(CanIdError::ExtendedOverflow);
        }
        Ok(Self {
            value,
            extended: true,
        })
    }
}

/// Frame kind — Classical CAN (≤ 8 bytes) or CAN-FD (≤ 64 bytes).
///
/// Determines [`sonic_connector_core::ChannelDescriptor::max_payload_size`]
/// deterministically per `REQ_0612`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum CanFrameKind {
    /// Classical CAN — 2.0A / 2.0B, ≤ 8 bytes payload.
    Classical,
    /// CAN-FD — flexible data-rate, ≤ 64 bytes payload, optional BRS
    /// / ESI flags.
    Fd,
}

impl CanFrameKind {
    /// Maximum payload bytes for this frame kind.
    ///
    /// * Classical → 8
    /// * Fd       → 64
    #[must_use]
    pub const fn max_payload(self) -> usize {
        match self {
            Self::Classical => 8,
            Self::Fd => 64,
        }
    }
}

bitflags! {
    /// CAN-FD-specific flags carried alongside the data bytes.
    ///
    /// Ignored when [`CanRouting::kind`] is
    /// [`CanFrameKind::Classical`].
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
    pub struct CanFdFlags: u8 {
        /// Bit-Rate Switch — payload phase runs at the FD data
        /// bitrate.
        const BRS = 0b0000_0001;
        /// Error State Indicator — transmitter is error-passive.
        const ESI = 0b0000_0010;
    }
}

/// Identifies one channel: which interface to bind to, which CAN ID
/// (and mask) to accept on inbound, which frame kind to construct on
/// outbound, and which FD flags to set.
///
/// Implements [`Routing`] (`REQ_0222`): `Clone + Send + Sync + Debug +
/// 'static`, no methods of its own.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CanRouting {
    /// Linux network interface this channel binds to.
    pub iface: CanIface,
    /// CAN identifier (with the standard / extended discriminant).
    pub can_id: CanId,
    /// Kernel-style filter mask. Matches `CAN_RAW_FILTER` semantics:
    /// a frame's ID matches when `(frame_id ^ can_id.value) & mask == 0`.
    pub mask: u32,
    /// Classical or FD; sets the channel's payload sizing per
    /// `REQ_0612`.
    pub kind: CanFrameKind,
    /// FD flags (BRS / ESI). Ignored when `kind == Classical`.
    pub fd_flags: CanFdFlags,
}

impl CanRouting {
    /// Construct a routing with [`CanFdFlags::empty`] FD flags.
    ///
    /// For FD channels with BRS / ESI, set the flags after construction
    /// or via [`Self::with_fd_flags`].
    #[must_use]
    pub const fn new(iface: CanIface, can_id: CanId, mask: u32, kind: CanFrameKind) -> Self {
        Self {
            iface,
            can_id,
            mask,
            kind,
            fd_flags: CanFdFlags::empty(),
        }
    }

    /// Builder-style FD flag setter.
    #[must_use]
    pub const fn with_fd_flags(mut self, flags: CanFdFlags) -> Self {
        self.fd_flags = flags;
        self
    }
}

impl Routing for CanRouting {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iface_round_trip() {
        let n = CanIface::new("vcan0").unwrap();
        assert_eq!(n.as_str(), "vcan0");
        assert_eq!(n.to_string(), "vcan0");
    }

    #[test]
    fn iface_rejects_empty_too_long_and_bad_chars() {
        assert_eq!(CanIface::new(""), Err(CanIfaceError::Empty));
        assert_eq!(
            CanIface::new("a-very-long-iface-name-x"),
            Err(CanIfaceError::TooLong)
        );
        assert_eq!(CanIface::new("can/0"), Err(CanIfaceError::InvalidChar));
        assert_eq!(CanIface::new("can 0"), Err(CanIfaceError::InvalidChar));
    }

    #[test]
    fn standard_id_caps_at_0x7ff() {
        assert!(CanId::standard(0x7FF).is_ok());
        assert_eq!(CanId::standard(0x800), Err(CanIdError::StandardOverflow));
    }

    #[test]
    fn extended_id_caps_at_29_bits() {
        assert!(CanId::extended(0x1FFF_FFFF).is_ok());
        assert_eq!(
            CanId::extended(0x2000_0000),
            Err(CanIdError::ExtendedOverflow)
        );
    }

    #[test]
    fn standard_and_extended_with_same_value_are_distinct() {
        let s = CanId::standard(0x123).unwrap();
        let e = CanId::extended(0x123).unwrap();
        assert_ne!(s, e);
        assert!(!s.extended);
        assert!(e.extended);
    }

    #[test]
    fn frame_kind_max_payload() {
        assert_eq!(CanFrameKind::Classical.max_payload(), 8);
        assert_eq!(CanFrameKind::Fd.max_payload(), 64);
    }

    #[test]
    fn routing_round_trips() {
        let iface = CanIface::new("vcan0").unwrap();
        let r = CanRouting::new(
            iface,
            CanId::standard(0x123).unwrap(),
            0x7FF,
            CanFrameKind::Classical,
        )
        .with_fd_flags(CanFdFlags::BRS);
        let r2 = r;
        assert_eq!(r, r2);
    }
}
