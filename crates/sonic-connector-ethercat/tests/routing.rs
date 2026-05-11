//! TEST_0201 — `EthercatRouting` carries the four addressing fields
//! verbatim. Verifies REQ_0311.

#![allow(clippy::doc_markdown)]

use sonic_connector_core::Routing;
use sonic_connector_ethercat::{EthercatRouting, PdoDirection};

/// Compile-time assertion: `EthercatRouting: Routing`. The `Routing`
/// trait carries no methods; this proves the marker bound holds.
const _: fn() = || {
    const fn assert_routing<R: Routing>() {}
    assert_routing::<EthercatRouting>();
};

#[test]
fn routing_round_trips_constructor_fields() {
    let r = EthercatRouting::new(0x0042, PdoDirection::Tx, 96, 16);
    assert_eq!(r.subdevice_address, 0x0042);
    assert_eq!(r.direction, PdoDirection::Tx);
    assert_eq!(r.bit_offset, 96);
    assert_eq!(r.bit_length, 16);
}

#[test]
fn routing_is_clone_eq_debug() {
    let r = EthercatRouting::new(7, PdoDirection::Rx, 0, 8);
    let s = r;
    assert_eq!(r, s);
    let dbg = format!("{r:?}");
    assert!(dbg.contains("Rx"));
    assert!(dbg.contains("subdevice_address"));
}

#[test]
fn rx_and_tx_directions_are_distinct() {
    let a = EthercatRouting::new(1, PdoDirection::Rx, 0, 8);
    let b = EthercatRouting::new(1, PdoDirection::Tx, 0, 8);
    assert_ne!(a.direction, b.direction);
    assert_ne!(a, b);
}
