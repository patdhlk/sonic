//! TEST_0207 — `CycleScheduler` exhibits miss-skip semantics:
//! advancing the clock by N intervals fires exactly one cycle, not
//! N (`REQ_0317`).

#![allow(clippy::doc_markdown)]

use std::time::{Duration, Instant};

use sonic_connector_ethercat::{CycleDecision, CycleScheduler};

#[test]
fn first_poll_always_fires() {
    let mut s = CycleScheduler::new(Duration::from_millis(10));
    let now = Instant::now();
    assert_eq!(s.poll(now), CycleDecision::Fire);
    assert_eq!(s.last_tick(), Some(now));
}

#[test]
fn poll_before_interval_elapsed_skips() {
    let cycle = Duration::from_millis(10);
    let mut s = CycleScheduler::new(cycle);
    let t0 = Instant::now();
    s.poll(t0);
    let t1 = t0 + Duration::from_millis(5);
    assert_eq!(s.poll(t1), CycleDecision::Skip);
    assert_eq!(s.last_tick(), Some(t0)); // last_tick unchanged
}

#[test]
fn poll_exactly_at_interval_fires() {
    let cycle = Duration::from_millis(10);
    let mut s = CycleScheduler::new(cycle);
    let t0 = Instant::now();
    s.poll(t0);
    let t1 = t0 + cycle;
    assert_eq!(s.poll(t1), CycleDecision::Fire);
    assert_eq!(s.last_tick(), Some(t1));
}

/// REQ_0317: when many intervals have elapsed between polls, only
/// ONE tick fires; the missed cycles are not queued for catch-up.
#[test]
fn large_jump_fires_once_not_many() {
    let cycle = Duration::from_millis(10);
    let mut s = CycleScheduler::new(cycle);
    let t0 = Instant::now();
    s.poll(t0);

    // Simulate 10 cycle-times of jitter.
    let jumped = t0 + cycle * 10;
    assert_eq!(s.poll(jumped), CycleDecision::Fire);
    assert_eq!(s.last_tick(), Some(jumped));

    // Immediate follow-up poll skips — the scheduler does NOT
    // "catch up" the 9 skipped cycles.
    assert_eq!(
        s.poll(jumped + Duration::from_micros(100)),
        CycleDecision::Skip
    );

    // After another full interval, one more tick fires.
    let later = jumped + cycle;
    assert_eq!(s.poll(later), CycleDecision::Fire);
}

#[test]
fn many_back_to_back_polls_yield_steady_cycle_count() {
    let cycle = Duration::from_millis(5);
    let mut s = CycleScheduler::new(cycle);
    let start = Instant::now();

    // 100 poll points spaced exactly one interval apart should yield
    // 100 fires (the very first poll fires at start).
    let mut fires = 0_u32;
    for i in 0..100 {
        let now = start + cycle * i;
        if s.poll(now) == CycleDecision::Fire {
            fires += 1;
        }
    }
    assert_eq!(fires, 100);
}
