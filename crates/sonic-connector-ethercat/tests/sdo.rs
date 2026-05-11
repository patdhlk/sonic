//! TEST_0205 (partial) — `pdo_sdo_writes` generates the ordered SDO
//! write sequence the gateway will issue during the PRE-OP → SAFE-OP
//! transition. Tests the sequence shape against `REQ_0315`; the
//! actual ethercrab-driven write path lands in C5c with bus-gated
//! integration tests.

#![allow(clippy::doc_markdown)]

use sonic_connector_ethercat::{
    PdoEntry, SM_ASSIGN_RX_PDO, SM_ASSIGN_TX_PDO, SdoValue, SdoWrite, SubDeviceMap, pdo_sdo_writes,
};

static EMPTY_ENTRIES: &[PdoEntry] = &[];

static TWO_RX_ENTRIES: &[PdoEntry] = &[
    PdoEntry {
        index: 0x1600,
        bit_offset: 0,
        bit_length: 16,
    },
    PdoEntry {
        index: 0x1601,
        bit_offset: 16,
        bit_length: 16,
    },
];

static THREE_TX_ENTRIES: &[PdoEntry] = &[
    PdoEntry {
        index: 0x1A00,
        bit_offset: 0,
        bit_length: 16,
    },
    PdoEntry {
        index: 0x1A01,
        bit_offset: 16,
        bit_length: 16,
    },
    PdoEntry {
        index: 0x1A02,
        bit_offset: 32,
        bit_length: 32,
    },
];

#[test]
fn empty_pdo_map_emits_no_writes() {
    let map = SubDeviceMap {
        address: 0x0001,
        rx_pdos: EMPTY_ENTRIES,
        tx_pdos: EMPTY_ENTRIES,
    };
    assert!(pdo_sdo_writes(&map).is_empty());
}

#[test]
fn rx_only_emits_clear_assign_count_to_index_0x1c12() {
    let map = SubDeviceMap {
        address: 0x0042,
        rx_pdos: TWO_RX_ENTRIES,
        tx_pdos: EMPTY_ENTRIES,
    };
    let writes = pdo_sdo_writes(&map);

    // 2 entries → 1 clear + 2 entry writes + 1 count = 4 writes.
    assert_eq!(writes.len(), 4);

    // All target the same SubDevice and the RxPDO sync-manager index.
    for w in &writes {
        assert_eq!(w.subdevice_address, 0x0042);
        assert_eq!(w.index, SM_ASSIGN_RX_PDO);
    }

    // Sequence shape: clear (0, 0u8), entry 1 (1, 0x1600u16),
    // entry 2 (2, 0x1601u16), count (0, 2u8).
    assert_eq!(
        writes[0],
        SdoWrite {
            subdevice_address: 0x0042,
            index: SM_ASSIGN_RX_PDO,
            subindex: 0,
            value: SdoValue::U8(0),
        }
    );
    assert_eq!(
        writes[1],
        SdoWrite {
            subdevice_address: 0x0042,
            index: SM_ASSIGN_RX_PDO,
            subindex: 1,
            value: SdoValue::U16(0x1600),
        }
    );
    assert_eq!(
        writes[2],
        SdoWrite {
            subdevice_address: 0x0042,
            index: SM_ASSIGN_RX_PDO,
            subindex: 2,
            value: SdoValue::U16(0x1601),
        }
    );
    assert_eq!(
        writes[3],
        SdoWrite {
            subdevice_address: 0x0042,
            index: SM_ASSIGN_RX_PDO,
            subindex: 0,
            value: SdoValue::U8(2),
        }
    );
}

#[test]
fn tx_only_emits_clear_assign_count_to_index_0x1c13() {
    let map = SubDeviceMap {
        address: 0x0007,
        rx_pdos: EMPTY_ENTRIES,
        tx_pdos: THREE_TX_ENTRIES,
    };
    let writes = pdo_sdo_writes(&map);

    // 3 entries → 1 + 3 + 1 = 5 writes.
    assert_eq!(writes.len(), 5);
    for w in &writes {
        assert_eq!(w.subdevice_address, 0x0007);
        assert_eq!(w.index, SM_ASSIGN_TX_PDO);
    }
    assert_eq!(writes[0].value, SdoValue::U8(0));
    assert_eq!(writes[1].value, SdoValue::U16(0x1A00));
    assert_eq!(writes[2].value, SdoValue::U16(0x1A01));
    assert_eq!(writes[3].value, SdoValue::U16(0x1A02));
    assert_eq!(writes[4].value, SdoValue::U8(3));
}

#[test]
fn both_directions_emit_rx_then_tx() {
    let map = SubDeviceMap {
        address: 0x0011,
        rx_pdos: TWO_RX_ENTRIES,
        tx_pdos: THREE_TX_ENTRIES,
    };
    let writes = pdo_sdo_writes(&map);

    // Rx (2 entries) → 4 writes, Tx (3 entries) → 5 writes = 9.
    assert_eq!(writes.len(), 9);

    // First four target Rx; remaining five target Tx.
    assert!(writes[..4].iter().all(|w| w.index == SM_ASSIGN_RX_PDO));
    assert!(writes[4..].iter().all(|w| w.index == SM_ASSIGN_TX_PDO));
}

/// Subindex numbering starts at 1 for entries and uses 0 only for
/// clear / set-count.
#[test]
fn entry_subindexes_start_at_one_and_count_writes_use_subindex_zero() {
    let map = SubDeviceMap {
        address: 0x00ff,
        rx_pdos: TWO_RX_ENTRIES,
        tx_pdos: EMPTY_ENTRIES,
    };
    let writes = pdo_sdo_writes(&map);
    let entry_subindexes: Vec<u8> = writes
        .iter()
        .filter_map(|w| matches!(w.value, SdoValue::U16(_)).then_some(w.subindex))
        .collect();
    let count_subindexes: Vec<u8> = writes
        .iter()
        .filter_map(|w| matches!(w.value, SdoValue::U8(_)).then_some(w.subindex))
        .collect();
    assert_eq!(entry_subindexes, vec![1, 2]);
    assert_eq!(count_subindexes, vec![0, 0]);
}
