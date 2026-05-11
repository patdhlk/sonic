//! Verifies that the forward-compatible
//! [`sonic_connector_ethercat::declare_pdu_storage!`] macro
//! expands cleanly and produces a `static` of the expected type.
//! Gated on the `bus-integration` feature so the test only runs
//! when the ethercrab dep is pulled.

#![cfg(feature = "bus-integration")]
#![allow(clippy::doc_markdown)]

use sonic_connector_ethercat::bus::{
    ETHERCAT_MAX_FRAMES, ETHERCAT_MAX_PDU_DATA, EthercatPduStorage,
};

sonic_connector_ethercat::declare_pdu_storage!(GATEWAY_A_STORAGE);
sonic_connector_ethercat::declare_pdu_storage!(GATEWAY_B_STORAGE);

#[test]
fn macro_declares_static_of_correct_type() {
    let _: &'static EthercatPduStorage = &GATEWAY_A_STORAGE;
    let _: &'static EthercatPduStorage = &GATEWAY_B_STORAGE;
}

#[test]
fn frame_pool_constants_match_recommended_defaults() {
    assert_eq!(ETHERCAT_MAX_FRAMES, 16);
    assert_eq!(ETHERCAT_MAX_PDU_DATA, 1100);
}
