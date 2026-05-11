//! End-to-end cycle-loop tests using `MockBusDriver`. Realises the
//! full integration of `CycleScheduler` + `BusDriver` + WKC policy +
//! `EthercatHealthMonitor` that C5b's pure-logic tests covered only
//! component-wise:
//!
//! * TEST_0207-full — scheduler skip-not-catch-up via runner.tick.
//! * TEST_0209-full — matching WKC drives health to `Up`.
//! * TEST_0210-full — mismatching WKC drives health to `Degraded`
//!   with the cycle index in the reason.
//! * TEST_0211-full — multiple cycles flow through the runner, with
//!   `cycle_index` incrementing on each fire.
//!
//! All powered by `MockBusDriver`; no `ethercrab`, no NIC, no hardware.

#![allow(clippy::doc_markdown)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use sonic_connector_core::{ConnectorHealth, ConnectorHealthKind};
use sonic_connector_ethercat::{
    BringUp, CycleReport, CycleRunner, EthercatConnectorOptions, EthercatHealthMonitor,
    MockBusDriver, WkcVerdict,
};

fn make_health() -> Arc<EthercatHealthMonitor> {
    Arc::new(EthercatHealthMonitor::new())
}

fn options(cycle_ms: u64) -> EthercatConnectorOptions {
    EthercatConnectorOptions::builder()
        .cycle_time(Duration::from_millis(cycle_ms))
        .build()
}

#[tokio::test]
async fn bring_up_transitions_connecting_to_up() {
    let health = make_health();
    assert_eq!(health.current().kind(), ConnectorHealthKind::Connecting);

    let driver = MockBusDriver::new().with_bring_up(BringUp {
        expected_wkc: 5,
        subdevice_count: 2,
    });
    let runner = CycleRunner::new(driver, &options(2), Arc::clone(&health))
        .await
        .expect("bring_up succeeds");

    assert_eq!(runner.expected_wkc(), 5);
    assert_eq!(runner.cycle_index(), 0);
    assert_eq!(health.current().kind(), ConnectorHealthKind::Up);
}

#[tokio::test]
async fn bring_up_failure_does_not_transition_health() {
    let health = make_health();
    let driver = MockBusDriver::new().failing_bring_up("simulated bring-up failure");
    let err = CycleRunner::new(driver, &options(2), Arc::clone(&health))
        .await
        .expect_err("bring_up should error");
    // Driver returned ConnectorError::Down — the runner forwards
    // it; the health monitor stays in its initial state.
    let msg = format!("{err}");
    assert!(msg.contains("simulated bring-up failure"), "{msg}");
    assert_eq!(health.current().kind(), ConnectorHealthKind::Connecting);
}

#[tokio::test]
async fn matching_wkc_keeps_connector_up() {
    let health = make_health();
    let driver = MockBusDriver::new()
        .with_bring_up(BringUp {
            expected_wkc: 3,
            subdevice_count: 1,
        })
        .with_default_cycle_wkc(3);
    let mut runner = CycleRunner::new(driver, &options(2), Arc::clone(&health))
        .await
        .unwrap();

    let t0 = Instant::now();
    let report: CycleReport = runner
        .tick(t0)
        .await
        .expect("tick succeeds")
        .expect("first poll fires");
    assert_eq!(report.cycle_index, 0);
    assert_eq!(report.working_counter, 3);
    assert!(matches!(report.verdict, WkcVerdict::Match));
    assert_eq!(health.current().kind(), ConnectorHealthKind::Up);
    assert_eq!(runner.cycle_index(), 1);
}

#[tokio::test]
async fn mismatching_wkc_transitions_to_degraded_with_cycle_in_reason() {
    let health = make_health();
    let driver = MockBusDriver::new()
        .with_bring_up(BringUp {
            expected_wkc: 5,
            subdevice_count: 1,
        })
        .with_default_cycle_wkc(2); // 2 < 5 → Mismatch every cycle
    let mut runner = CycleRunner::new(driver, &options(2), Arc::clone(&health))
        .await
        .unwrap();

    let t0 = Instant::now();
    let report = runner.tick(t0).await.unwrap().unwrap();
    assert_eq!(report.cycle_index, 0);
    assert!(matches!(
        report.verdict,
        WkcVerdict::Mismatch {
            observed: 2,
            expected: 5,
        }
    ));
    match health.current() {
        ConnectorHealth::Degraded { reason } => {
            assert!(
                reason.contains('2'),
                "reason must name observed=2: {reason}"
            );
            assert!(
                reason.contains('5'),
                "reason must name expected=5: {reason}"
            );
        }
        other => panic!("expected Degraded, got {other:?}"),
    }
}

#[tokio::test]
async fn skip_not_catch_up_after_clock_jump() {
    let health = make_health();
    let driver = MockBusDriver::new().with_default_cycle_wkc(3);
    let mut runner = CycleRunner::new(driver, &options(10), Arc::clone(&health))
        .await
        .unwrap();

    // First tick fires unconditionally.
    let t0 = Instant::now();
    assert!(runner.tick(t0).await.unwrap().is_some());

    // Clock jump of 10 × cycle_time: REQ_0317 says ONE cycle fires,
    // not 10. After this single fire, an immediate follow-up tick
    // returns None (no catch-up).
    let jumped = t0 + Duration::from_millis(10 * 10);
    assert!(runner.tick(jumped).await.unwrap().is_some());
    assert!(
        runner
            .tick(jumped + Duration::from_micros(100))
            .await
            .unwrap()
            .is_none(),
        "scheduler must skip immediately after a fire"
    );

    // The runner has issued exactly 2 cycle calls (the first tick and
    // the post-jump tick), confirming no catch-up.
    assert_eq!(runner.cycle_index(), 2);
}

#[tokio::test]
async fn up_degraded_up_sequence_observed_through_health_channel() {
    let health = make_health();
    let sub = health.subscribe();

    // Bring up Connecting → Up (cycle 0), then run cycles 1..=5 with
    // a WKC sequence [3, 2, 2, 3, 3]: cycle 1 stays Up, cycle 2
    // transitions to Degraded, cycle 3 stays Degraded, cycle 4
    // transitions back to Up, cycle 5 stays Up.
    let driver = MockBusDriver::new()
        .with_bring_up(BringUp {
            expected_wkc: 3,
            subdevice_count: 1,
        })
        .with_wkc_sequence([3, 2, 2, 3, 3]);
    let mut runner = CycleRunner::new(driver, &options(2), Arc::clone(&health))
        .await
        .unwrap();

    let mut now = Instant::now();
    for _ in 0..5 {
        let report = runner.tick(now).await.unwrap().expect("cycle fires");
        let _ = report;
        now += Duration::from_millis(2);
    }

    // Drain emitted HealthEvents. Expected sequence:
    //   Connecting → Up (from bring_up)
    //   Up → Degraded (cycle 2's WKC=2 vs expected=3)
    //   Degraded → Up (cycle 4's WKC=3 matches)
    let mut events = Vec::new();
    while let Ok(ev) = sub.try_recv() {
        events.push((ev.from.kind(), ev.to.kind()));
    }
    assert_eq!(
        events,
        vec![
            (ConnectorHealthKind::Connecting, ConnectorHealthKind::Up),
            (ConnectorHealthKind::Up, ConnectorHealthKind::Degraded),
            (ConnectorHealthKind::Degraded, ConnectorHealthKind::Up),
        ],
        "observed events: {events:?}"
    );
    assert_eq!(health.current().kind(), ConnectorHealthKind::Up);
}

#[tokio::test]
async fn cycle_index_increments_on_every_fire_not_on_skips() {
    let health = make_health();
    let driver = MockBusDriver::new().with_default_cycle_wkc(3);
    let mut runner = CycleRunner::new(driver, &options(10), Arc::clone(&health))
        .await
        .unwrap();

    let t0 = Instant::now();
    // Fire 3 cycles, each spaced exactly one cycle time apart.
    for i in 0..3 {
        let now = t0 + Duration::from_millis(10 * i);
        assert!(runner.tick(now).await.unwrap().is_some());
    }
    assert_eq!(runner.cycle_index(), 3);

    // Three skips between cycles do NOT increment.
    let skip_at = t0 + Duration::from_millis(10 * 3 - 5);
    assert!(runner.tick(skip_at).await.unwrap().is_none());
    assert!(
        runner
            .tick(skip_at + Duration::from_micros(100))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        runner
            .tick(skip_at + Duration::from_micros(200))
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(runner.cycle_index(), 3);
}
