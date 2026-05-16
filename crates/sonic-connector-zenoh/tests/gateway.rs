//! Tests for `ZenohGateway` — the tokio runtime owner. Mirrors the
//! shape of `sonic_connector_ethercat`'s gateway tests.

use std::time::Duration;

use sonic_connector_zenoh::ZenohConnectorOptions;
use sonic_connector_zenoh::gateway::{DEFAULT_SHUTDOWN_BUDGET, ZenohGateway};

#[test]
fn gateway_starts_a_runtime() {
    let opts = ZenohConnectorOptions::builder().build();
    let gateway = ZenohGateway::new(opts).expect("runtime up");
    // Spawn a trivial task and block until it completes — proves the
    // runtime is up without naming any `tokio::` type (`REQ_0403`).
    let (tx, rx) = std::sync::mpsc::channel::<u32>();
    gateway.spawn(async move {
        tx.send(42).unwrap();
    });
    let v = rx.recv_timeout(Duration::from_secs(1)).expect("ran");
    assert_eq!(v, 42);
}

#[test]
fn default_shutdown_budget_is_five_seconds() {
    let opts = ZenohConnectorOptions::builder().build();
    let gateway = ZenohGateway::new(opts).expect("runtime up");
    assert_eq!(gateway.shutdown_budget(), DEFAULT_SHUTDOWN_BUDGET);
    assert_eq!(DEFAULT_SHUTDOWN_BUDGET, Duration::from_secs(5));
}

#[test]
fn custom_shutdown_budget() {
    let opts = ZenohConnectorOptions::builder().build();
    let gateway =
        ZenohGateway::with_shutdown_budget(opts, Duration::from_millis(250)).expect("runtime up");
    assert_eq!(gateway.shutdown_budget(), Duration::from_millis(250));
}

#[test]
fn dropping_gateway_joins_runtime_within_budget() {
    use std::time::Instant;
    let opts = ZenohConnectorOptions::builder().build();
    let gateway =
        ZenohGateway::with_shutdown_budget(opts, Duration::from_millis(500)).expect("runtime up");
    let start = Instant::now();
    drop(gateway);
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "drop took {elapsed:?}, expected to honour 500ms budget"
    );
}

#[test]
fn worker_thread_count_honored() {
    let opts = ZenohConnectorOptions::builder()
        .tokio_worker_threads(2)
        .build();
    let gateway = ZenohGateway::new(opts).expect("runtime up");
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    gateway.spawn(async move {
        tx.send(()).unwrap();
    });
    rx.recv_timeout(Duration::from_secs(1))
        .expect("ran on multi-worker");
}
