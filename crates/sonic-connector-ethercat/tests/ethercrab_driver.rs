//! Hardware-gated `EthercrabBusDriver` integration test. Gated on
//! the `bus-integration` cargo feature; marked `#[ignore]` so it does
//! not run during normal `cargo test` invocations.
//!
//! ## Running against real hardware
//!
//! ```sh
//! # Linux gateway host, network interface `eth0`, requires CAP_NET_RAW
//! ETHERCAT_TEST_NIC=eth0 \
//!   cargo test -p sonic-connector-ethercat \
//!     --features bus-integration \
//!     --test ethercrab_driver \
//!     -- --ignored --test-threads=1
//! ```
//!
//! Tests that require `ETHERCAT_TEST_NIC` shall return early with a
//! clear log message when the env var is absent so the test runner
//! doesn't surface them as failures during CI or local-dev runs that
//! happen to enable the `bus-integration` feature.

#![cfg(feature = "bus-integration")]
#![allow(clippy::doc_markdown)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use sonic_connector_core::ConnectorHealthKind;
use sonic_connector_ethercat::{
    CycleRunner, EthercatConnectorOptions, EthercatHealthMonitor, EthercrabBusDriver,
    declare_pdu_storage,
};

declare_pdu_storage!(TEST_PDU_STORAGE);

const MAX_SUBDEVICES: usize = 16;
const MAX_PDI: usize = 256;

fn maybe_nic() -> Option<String> {
    let iface = std::env::var("ETHERCAT_TEST_NIC").ok()?;
    if iface.trim().is_empty() {
        None
    } else {
        Some(iface)
    }
}

/// Bus comes up to OP, cycle returns a working counter, health
/// transitions Connecting → Up. Requires a Linux host with an actual
/// EtherCAT bus on `ETHERCAT_TEST_NIC`.
#[tokio::test]
#[ignore = "requires real EtherCAT NIC; set ETHERCAT_TEST_NIC=<iface> and run with --ignored"]
async fn bring_up_and_cycle_against_real_bus() {
    let Some(iface) = maybe_nic() else {
        eprintln!("ETHERCAT_TEST_NIC not set; skipping");
        return;
    };

    let options = EthercatConnectorOptions::builder()
        .network_interface(iface)
        .cycle_time(Duration::from_millis(2))
        .build();

    let driver =
        EthercrabBusDriver::<MAX_SUBDEVICES, MAX_PDI>::new(&TEST_PDU_STORAGE, options.clone())
            .expect("driver construction");
    let health = Arc::new(EthercatHealthMonitor::new());

    // Box::pin because CycleRunner::new captures the full bring-up
    // future, which carries a sizeable SubDeviceGroup; pinning it
    // on the heap keeps the stack frame manageable.
    let mut runner = Box::pin(CycleRunner::new(driver, &options, Arc::clone(&health)))
        .await
        .expect("bring-up succeeds");

    assert_eq!(
        health.current().kind(),
        ConnectorHealthKind::Up,
        "bring-up should transition Connecting → Up"
    );

    // Run a handful of cycles; each tick should fire (cycle_time
    // elapsed between iterations) and return SOME working counter
    // ≥ 0. A more thorough test would assert specific WKC values
    // against known SubDevice topology.
    let mut now = Instant::now();
    for _ in 0..5 {
        let report = runner
            .tick(now)
            .await
            .expect("cycle succeeds")
            .expect("scheduler fires");
        eprintln!(
            "cycle {}: WKC={}",
            report.cycle_index, report.working_counter
        );
        now += options.cycle_time();
    }
}
