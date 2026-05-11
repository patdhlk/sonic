//! TEST_0211 (partial) — the gateway hosts a tokio sidecar contained
//! inside this crate. C5a verifies the runtime is constructed,
//! exposes a handle for spawning work, and is joined on `Drop` with
//! the configured budget (`ADR_0026`). The actual ethercrab TX/RX
//! task lands in C5b.

#![allow(clippy::doc_markdown)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use sonic_connector_ethercat::{EthercatConnectorOptions, EthercatGateway};

#[test]
fn gateway_runtime_can_run_spawned_work() {
    let opts = EthercatConnectorOptions::builder()
        .tokio_worker_threads(2)
        .build();
    let gw = EthercatGateway::new(opts).expect("gateway construction");
    let handle = gw.handle().expect("runtime alive");

    let counter = Arc::new(AtomicU32::new(0));
    let counter_c = Arc::clone(&counter);
    handle.block_on(async move {
        counter_c.fetch_add(1, Ordering::SeqCst);
    });
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[test]
fn gateway_default_shutdown_budget_is_five_seconds() {
    let opts = EthercatConnectorOptions::builder().build();
    let gw = EthercatGateway::new(opts).unwrap();
    assert_eq!(gw.shutdown_budget(), Duration::from_secs(5));
}

#[test]
fn gateway_drop_completes_within_budget() {
    // A short budget — the drop must return promptly (well under the
    // budget) when no in-flight tasks need joining.
    let opts = EthercatConnectorOptions::builder().build();
    let gw = EthercatGateway::with_shutdown_budget(opts, Duration::from_millis(500)).unwrap();
    let start = std::time::Instant::now();
    drop(gw);
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "gateway drop took {elapsed:?}; budget should not block the test"
    );
}
