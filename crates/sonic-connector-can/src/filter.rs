//! [`PerIfaceFilter`] — compiles the union of inbound channel masks
//! into kernel `CAN_RAW_FILTER` form and provides the symmetric
//! match predicate the dispatcher's RX demux uses. `BB_0074`,
//! `REQ_0622`, `REQ_0623`, `REQ_0624`.
//!
//! The compiler is intentionally idempotent — repeated calls with the
//! same set of routing tuples produce byte-identical filter arrays.
//! Deduplication is mask-aware: `(0x123, 0x7FF, std)` and
//! `(0x123, 0x7FF, ext)` are distinct because the kernel encodes the
//! standard / extended discriminant in the `CAN_EFF_FLAG` bit
//! (`REQ_0615`).

use crate::driver::{CAN_EFF_FLAG, CanData, CanFilter, CanFrame};
use crate::routing::CanRouting;

/// Compile a set of inbound `CanRouting` entries into a deduplicated
/// vector of kernel-style filters. Order is preserved from input —
/// callers that need a stable order should pass routing tuples in the
/// desired sequence.
#[must_use]
pub fn compile(routings: impl IntoIterator<Item = CanRouting>) -> PerIfaceFilter {
    let mut filters = Vec::new();
    for r in routings {
        let f = filter_for(&r);
        if !filters.contains(&f) {
            filters.push(f);
        }
    }
    PerIfaceFilter { filters }
}

/// Convert a single [`CanRouting`] to the kernel filter form.
#[must_use]
pub fn filter_for(r: &CanRouting) -> CanFilter {
    let eff = if r.can_id.extended { CAN_EFF_FLAG } else { 0 };
    // Fold the extended flag into can_id so the match predicate is a
    // single `(frame_id ^ can_id) & can_mask == 0`. The mask gets the
    // CAN_EFF_FLAG bit too so standard / extended discriminate.
    CanFilter {
        can_id: r.can_id.value | eff,
        can_mask: r.mask | CAN_EFF_FLAG,
    }
}

/// Compiled filter set for one interface.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PerIfaceFilter {
    filters: Vec<CanFilter>,
}

impl PerIfaceFilter {
    /// Construct from a pre-built vector. Skip the dedup pass; the
    /// caller is asserting uniqueness. Use [`compile`] for the safe
    /// path.
    #[must_use]
    pub const fn from_raw(filters: Vec<CanFilter>) -> Self {
        Self { filters }
    }

    /// Borrow the filter vector for handoff to `setsockopt`.
    #[must_use]
    pub fn as_slice(&self) -> &[CanFilter] {
        &self.filters
    }

    /// Number of distinct filter entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.filters.len()
    }

    /// `true` when no filters are registered (no inbound channels
    /// for this iface).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.filters.is_empty()
    }
}

/// Match a frame against a routing tuple. Returns `true` when the
/// frame's identifier and extended discriminant pass the routing's
/// `(can_id, mask)` filter under kernel `CAN_RAW_FILTER` semantics.
///
/// Mirrors the kernel's per-filter check:
///
/// ```text
///   (frame_id ^ filter.can_id) & filter.can_mask == 0
/// ```
///
/// with the `CAN_EFF_FLAG` bit folded into both id and mask so
/// standard / extended frames are not cross-matched (`REQ_0615`).
#[must_use]
pub fn matches(routing: &CanRouting, frame: &CanData) -> bool {
    if routing.can_id.extended != frame.id.extended {
        return false;
    }
    let mask = routing.mask;
    (routing.can_id.value ^ frame.id.value) & mask == 0
}

/// Match against a `CanFrame::Data` variant — convenience wrapper.
#[must_use]
pub fn matches_frame(routing: &CanRouting, frame: &CanFrame) -> bool {
    match frame {
        CanFrame::Data(d) => matches(routing, d),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::{CanFdFlags, CanFrameKind, CanId, CanIface};

    fn routing(iface: &str, id: u16, mask: u32, ext: bool) -> CanRouting {
        let iface = CanIface::new(iface).unwrap();
        let can_id = if ext {
            CanId::extended(u32::from(id)).unwrap()
        } else {
            CanId::standard(id).unwrap()
        };
        CanRouting::new(iface, can_id, mask, CanFrameKind::Classical)
    }

    fn data(id: u16, ext: bool, bytes: &[u8]) -> CanData {
        let can_id = if ext {
            CanId::extended(u32::from(id)).unwrap()
        } else {
            CanId::standard(id).unwrap()
        };
        CanData::new(can_id, CanFrameKind::Classical, CanFdFlags::empty(), bytes).unwrap()
    }

    #[test]
    fn compile_dedups_identical_routings() {
        let a = routing("vcan0", 0x100, 0x7FF, false);
        let b = routing("vcan0", 0x100, 0x7FF, false);
        let f = compile([a, b]);
        assert_eq!(f.len(), 1);
    }

    #[test]
    fn compile_distinguishes_standard_and_extended() {
        let s = routing("vcan0", 0x100, 0x7FF, false);
        let e = routing("vcan0", 0x100, 0x7FF, true);
        let f = compile([s, e]);
        assert_eq!(f.len(), 2);
    }

    #[test]
    fn matches_exact_id() {
        let r = routing("vcan0", 0x123, 0x7FF, false);
        assert!(matches(&r, &data(0x123, false, &[])));
        assert!(!matches(&r, &data(0x124, false, &[])));
    }

    #[test]
    fn matches_mask() {
        // mask 0x7F0 → match 0x120..=0x12F.
        let r = routing("vcan0", 0x120, 0x7F0, false);
        assert!(matches(&r, &data(0x120, false, &[])));
        assert!(matches(&r, &data(0x12F, false, &[])));
        assert!(!matches(&r, &data(0x110, false, &[])));
        assert!(!matches(&r, &data(0x130, false, &[])));
    }

    #[test]
    fn standard_and_extended_with_same_value_dont_cross_match() {
        let s = routing("vcan0", 0x123, 0x7FF, false);
        let e_frame = data(0x123, true, &[]);
        assert!(!matches(&s, &e_frame));
    }

    #[test]
    fn filter_for_sets_eff_flag_on_extended() {
        let r_std = routing("vcan0", 0x123, 0x7FF, false);
        let r_ext = routing("vcan0", 0x123, 0x7FF, true);
        let f_std = filter_for(&r_std);
        let f_ext = filter_for(&r_ext);
        assert_eq!(f_std.can_id & CAN_EFF_FLAG, 0);
        assert_eq!(f_ext.can_id & CAN_EFF_FLAG, CAN_EFF_FLAG);
        // Mask always sets EFF_FLAG so the discriminant participates.
        assert_eq!(f_std.can_mask & CAN_EFF_FLAG, CAN_EFF_FLAG);
        assert_eq!(f_ext.can_mask & CAN_EFF_FLAG, CAN_EFF_FLAG);
    }
}
