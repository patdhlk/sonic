//! SDO write sequence for applying a [`SubDeviceMap`]'s PDO assignment
//! during the PRE-OP → SAFE-OP transition. `REQ_0315`.
//!
//! The sequence for one direction (RxPDO via `0x1C12`, TxPDO via
//! `0x1C13`) is fixed by CoE:
//!
//! 1. Write `0u8` to subindex 0 — clears the current count.
//! 2. For each entry, write the entry's `index` (`u16`) to subindex
//!    `1, 2, 3, …`.
//! 3. Write the entry count back to subindex 0 (`u8`).
//!
//! This module emits an iterator of [`SdoWrite`] values describing
//! exactly that sequence. Sequencing is pure-logic so it can be
//! unit-tested without `ethercrab` or a real bus.

use crate::options::{PdoEntry, SubDeviceMap};

/// Sync-manager PDO assignment index for the RxPDO direction.
pub const SM_ASSIGN_RX_PDO: u16 = 0x1C12;
/// Sync-manager PDO assignment index for the TxPDO direction.
pub const SM_ASSIGN_TX_PDO: u16 = 0x1C13;

/// One SDO write — index / subindex / value triple plus target.
///
/// Carries the SubDevice configured address; the (ethercrab-backed)
/// gateway translates each [`SdoWrite`] into one
/// `subdevice.sdo_write(...)` call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SdoWrite {
    /// SubDevice configured address on the EtherCAT bus.
    pub subdevice_address: u16,
    /// SDO object dictionary index.
    pub index: u16,
    /// SDO object dictionary subindex.
    pub subindex: u8,
    /// Value to write. PDO assignment uses only `u8` (count) and
    /// `u16` (entry-index) values; broader types are not needed for
    /// `REQ_0315`.
    pub value: SdoValue,
}

/// SDO write value. The two variants cover the entire PDO-assignment
/// sequence (`REQ_0315`); broader types can land alongside future
/// SDO-based configuration paths.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SdoValue {
    /// 8-bit unsigned. Used for the count subindex of a PDO
    /// assignment.
    U8(u8),
    /// 16-bit unsigned. Used for each entry of a PDO assignment.
    U16(u16),
}

/// Emit the full SDO write sequence for one [`SubDeviceMap`].
///
/// Empty RxPDO / TxPDO lists produce no writes for that direction —
/// not even a clear-count, since the SubDevice's default mapping is
/// the desired state when the application has nothing to assign.
#[must_use]
pub fn pdo_sdo_writes(map: &SubDeviceMap) -> Vec<SdoWrite> {
    let mut out = Vec::new();
    push_direction(&mut out, map.address, SM_ASSIGN_RX_PDO, map.rx_pdos);
    push_direction(&mut out, map.address, SM_ASSIGN_TX_PDO, map.tx_pdos);
    out
}

fn push_direction(out: &mut Vec<SdoWrite>, address: u16, sm_index: u16, entries: &[PdoEntry]) {
    if entries.is_empty() {
        return;
    }

    // Step 1 — clear count.
    out.push(SdoWrite {
        subdevice_address: address,
        index: sm_index,
        subindex: 0,
        value: SdoValue::U8(0),
    });

    // Step 2 — assign each entry. Subindex starts at 1.
    for (i, entry) in entries.iter().enumerate() {
        let subindex = u8::try_from(i + 1)
            .expect("PDO entries fit in u8 subindex; SubDeviceMap is a static slice ≤ 255");
        out.push(SdoWrite {
            subdevice_address: address,
            index: sm_index,
            subindex,
            value: SdoValue::U16(entry.index),
        });
    }

    // Step 3 — set count.
    let count = u8::try_from(entries.len())
        .expect("PDO entries fit in u8 count; SubDeviceMap is a static slice ≤ 255");
    out.push(SdoWrite {
        subdevice_address: address,
        index: sm_index,
        subindex: 0,
        value: SdoValue::U8(count),
    });
}
