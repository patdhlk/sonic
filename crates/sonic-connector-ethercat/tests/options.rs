//! TEST_0204 (static PDO map accepted from options) and TEST_0206
//! (cycle time configurable with millisecond resolution).

#![allow(clippy::doc_markdown)]

use std::time::Duration;

use sonic_connector_ethercat::{EthercatConnectorOptions, PdoEntry, SubDeviceMap};

/// `'static` PDO map declared in test code — mirrors application usage
/// per ADR_0027.
static RX_ENTRIES: &[PdoEntry] = &[PdoEntry {
    index: 0x6040,
    bit_offset: 0,
    bit_length: 16,
}];
static TX_ENTRIES: &[PdoEntry] = &[PdoEntry {
    index: 0x6041,
    bit_offset: 0,
    bit_length: 16,
}];
static PDO_MAP: &[SubDeviceMap] = &[SubDeviceMap {
    address: 0x0001,
    rx_pdos: RX_ENTRIES,
    tx_pdos: TX_ENTRIES,
}];

#[test]
fn default_options_match_spec() {
    let opts = EthercatConnectorOptions::builder().build();
    // REQ_0316 default cycle time is 2 ms.
    assert_eq!(opts.cycle_time(), Duration::from_millis(2));
    // REQ_0318 default DC bring-up is opt-in (off by default).
    assert!(!opts.distributed_clocks());
    // Bridge capacities default to 256 (REQ_0322 — value not in spec,
    // chosen sensibly).
    assert_eq!(opts.outbound_capacity(), 256);
    assert_eq!(opts.inbound_capacity(), 256);
    // Empty PDO map by default.
    assert!(opts.pdo_map().is_empty());
}

#[test]
fn pdo_map_round_trips_through_builder() {
    let opts = EthercatConnectorOptions::builder().pdo_map(PDO_MAP).build();
    let observed = opts.pdo_map();
    assert_eq!(observed.len(), 1);
    assert_eq!(observed[0].address, 0x0001);
    assert_eq!(observed[0].rx_pdos.len(), 1);
    assert_eq!(observed[0].tx_pdos.len(), 1);
    assert_eq!(observed[0].rx_pdos[0].index, 0x6040);
    assert_eq!(observed[0].tx_pdos[0].index, 0x6041);
}

#[test]
fn cycle_time_overrides_default_with_millisecond_resolution() {
    let opts = EthercatConnectorOptions::builder()
        .cycle_time(Duration::from_millis(5))
        .build();
    assert_eq!(opts.cycle_time(), Duration::from_millis(5));
}

#[test]
fn cycle_time_clamps_to_one_millisecond_minimum() {
    let opts = EthercatConnectorOptions::builder()
        .cycle_time(Duration::from_micros(500))
        .build();
    // REQ_0316 says minimum resolution is 1 ms — submilli requests are
    // clamped up to 1 ms.
    assert_eq!(opts.cycle_time(), Duration::from_millis(1));
}

#[test]
fn distributed_clocks_opt_in() {
    let off = EthercatConnectorOptions::builder().build();
    let on = EthercatConnectorOptions::builder()
        .distributed_clocks(true)
        .build();
    assert!(!off.distributed_clocks());
    assert!(on.distributed_clocks());
}

#[test]
fn bridge_capacities_clamp_to_one_minimum() {
    let opts = EthercatConnectorOptions::builder()
        .outbound_capacity(0)
        .inbound_capacity(0)
        .build();
    assert_eq!(opts.outbound_capacity(), 1);
    assert_eq!(opts.inbound_capacity(), 1);
}

#[test]
fn tokio_worker_threads_clamps_to_one_minimum() {
    let opts = EthercatConnectorOptions::builder()
        .tokio_worker_threads(0)
        .build();
    assert_eq!(opts.tokio_worker_threads(), 1);
}

#[test]
fn network_interface_round_trips() {
    let opts = EthercatConnectorOptions::builder()
        .network_interface("eth0")
        .build();
    assert_eq!(opts.network_interface(), Some("eth0"));
}
