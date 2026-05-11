//! TEST_0100 — `ExponentialBackoff` invariants. Verifies REQ_0233:
//! delays are monotonically non-decreasing until the cap, never exceed
//! the configured maximum, `reset()` returns to the initial delay, and
//! jitter stays within the configured ratio.

#![allow(clippy::doc_markdown)]

use std::time::Duration;

use proptest::prelude::*;
use sonic_connector_core::{ExponentialBackoff, ReconnectPolicy};

/// Without jitter, every successive delay is `> previous` until it
/// reaches the cap, after which every delay equals the cap exactly.
#[test]
fn delays_are_monotonically_non_decreasing_until_cap() {
    let mut b = ExponentialBackoff::builder()
        .initial(Duration::from_millis(10))
        .max(Duration::from_millis(80))
        .growth(2.0)
        .jitter(0.0)
        .build();

    let mut prev = Duration::from_millis(0);
    let mut at_cap = false;
    for _ in 0..20 {
        let d = b.next_delay();
        if at_cap {
            assert_eq!(d, Duration::from_millis(80), "post-cap delay drifted");
        } else {
            assert!(d >= prev, "delay shrank: {d:?} after {prev:?}");
            if d == Duration::from_millis(80) {
                at_cap = true;
            }
        }
        prev = d;
    }
}

/// Even with maximum jitter, delays never exceed `max`. The base before
/// jitter is bounded by `max`; jitter `* (1 + r)` with `r ∈ [0, 1]`
/// could overshoot, so the implementation must clamp.
#[test]
fn delays_never_exceed_max_even_with_jitter() {
    let max = Duration::from_millis(50);
    let mut b = ExponentialBackoff::builder()
        .initial(Duration::from_millis(1))
        .max(max)
        .growth(2.0)
        .jitter(1.0)
        .seed(0xDEAD_BEEF)
        .build();
    for _ in 0..200 {
        let d = b.next_delay();
        assert!(d <= max, "delay {d:?} exceeded max {max:?}");
    }
}

/// `reset()` brings the policy back to its initial delay regardless of
/// how many attempts have been issued.
#[test]
fn reset_returns_to_initial_delay() {
    let mut b = ExponentialBackoff::builder()
        .initial(Duration::from_millis(7))
        .max(Duration::from_secs(60))
        .growth(2.0)
        .jitter(0.0)
        .build();
    let initial = b.next_delay();
    for _ in 0..10 {
        let _ = b.next_delay();
    }
    b.reset();
    assert_eq!(b.next_delay(), initial);
}

proptest! {
    /// Property: the i-th delay (no jitter) is `min(initial * growth^i, max)`.
    #[test]
    fn base_delay_matches_geometric_formula(
        initial_ms in 1u64..1000,
        growth_x10 in 11u32..50,  // growth in [1.1, 5.0]
        attempts in 0u32..16,
    ) {
        let initial = Duration::from_millis(initial_ms);
        let max = Duration::from_secs(60);
        let growth = f64::from(growth_x10) / 10.0;
        let mut b = ExponentialBackoff::builder()
            .initial(initial)
            .max(max)
            .growth(growth)
            .jitter(0.0)
            .build();
        for _ in 0..attempts {
            let _ = b.next_delay();
        }
        let observed = b.next_delay();
        let expected_secs = initial.as_secs_f64() * growth.powf(f64::from(attempts));
        let expected = if expected_secs.is_finite() && expected_secs < max.as_secs_f64() {
            Duration::from_secs_f64(expected_secs)
        } else {
            max
        };
        // Floating-point: allow a 1-microsecond slack.
        let diff = observed.abs_diff(expected);
        prop_assert!(diff <= Duration::from_micros(1),
            "attempt {attempts}: observed {observed:?} vs expected {expected:?}");
    }

    /// Property: with jitter `j ∈ [0, 1]`, the returned delay lies in
    /// `[base*(1-j), min(base*(1+j), max)]`. We sample many seeds so the
    /// jitter distribution is exercised.
    #[test]
    fn jitter_stays_within_ratio(
        initial_ms in 1u64..1000,
        jitter_x100 in 0u32..101,  // jitter in [0.0, 1.0]
        seed in any::<u64>(),
    ) {
        let initial = Duration::from_millis(initial_ms);
        let max = Duration::from_secs(60);
        let jitter = f64::from(jitter_x100) / 100.0;
        let b = ExponentialBackoff::builder()
            .initial(initial)
            .max(max)
            .growth(2.0)
            .jitter(jitter)
            .seed(seed)
            .build();
        for _ in 0..8 {
            let base = b.base_delay_for_attempt(0);  // base for first attempt
            // re-instantiate so we measure only the first call's jitter
            let mut local: ExponentialBackoff = ExponentialBackoff::builder()
                .initial(initial)
                .max(max)
                .growth(2.0)
                .jitter(jitter)
                .seed(seed.wrapping_add(1))
                .build();
            let d = local.next_delay();
            let lower = Duration::from_secs_f64((base.as_secs_f64() * (1.0 - jitter)).max(0.0));
            let upper_unclamped = base.as_secs_f64() * (1.0 + jitter);
            let upper = Duration::from_secs_f64(upper_unclamped.min(max.as_secs_f64()));
            prop_assert!(d >= lower && d <= upper,
                "delay {d:?} outside [{lower:?}, {upper:?}] for base {base:?} jitter {jitter}");
        }
    }
}
