//! [`ReconnectPolicy`] — `REQ_0232` — and [`ExponentialBackoff`] —
//! `REQ_0233`, the default backoff used by every connector whose
//! protocol stack exposes raw connect / disconnect events.
//!
//! Stacks that handle reconnect internally (tonic-managed gRPC, MQTT
//! libraries with built-in retry) are not required to use this; they
//! still emit `HealthEvent` per `REQ_0235`.

use std::time::Duration;

use rand::{Rng, SeedableRng, rngs::StdRng};

/// Policy that schedules the gap between reconnect attempts.
pub trait ReconnectPolicy: Send {
    /// Return the delay before the next reconnect attempt and advance
    /// the policy's internal state. Subsequent calls without
    /// [`Self::reset`] shall return a delay greater than or equal to
    /// the previous one, up to the policy's configured maximum.
    fn next_delay(&mut self) -> Duration;

    /// Reset internal state so the next [`Self::next_delay`] returns
    /// the initial delay.
    fn reset(&mut self);
}

/// Builder for [`ExponentialBackoff`] — pulled out so the fields can
/// stay private on the policy itself.
#[derive(Clone, Debug)]
pub struct ExponentialBackoffBuilder {
    initial: Duration,
    max: Duration,
    growth: f64,
    jitter: f64,
    seed: Option<u64>,
}

impl ExponentialBackoffBuilder {
    /// Construct a builder with the default initial delay (100 ms),
    /// maximum delay (30 s), growth factor (2.0), and jitter ratio
    /// (0.1).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            initial: Duration::from_millis(100),
            max: Duration::from_secs(30),
            growth: 2.0,
            jitter: 0.1,
            seed: None,
        }
    }

    /// Override the initial delay. Must be positive.
    #[must_use]
    pub const fn initial(mut self, d: Duration) -> Self {
        self.initial = d;
        self
    }

    /// Override the cap on per-attempt delay.
    #[must_use]
    pub const fn max(mut self, d: Duration) -> Self {
        self.max = d;
        self
    }

    /// Override the growth factor (must be >= 1.0; smaller values are
    /// clamped at construction time).
    #[must_use]
    pub const fn growth(mut self, factor: f64) -> Self {
        self.growth = factor;
        self
    }

    /// Override the jitter ratio (0.0..=1.0). `0.1` means the returned
    /// delay is `base * (1 ± 0.1)`.
    #[must_use]
    pub const fn jitter(mut self, ratio: f64) -> Self {
        self.jitter = ratio;
        self
    }

    /// Seed the internal RNG so jitter is deterministic for tests.
    /// Production code should not set a seed.
    #[must_use]
    pub const fn seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }

    /// Build the policy. Invalid parameters are clamped:
    ///
    /// * `growth < 1.0` is clamped to `1.0`.
    /// * `jitter < 0.0` is clamped to `0.0`.
    /// * `jitter > 1.0` is clamped to `1.0`.
    /// * `max < initial` is clamped to `initial`.
    #[must_use]
    pub fn build(self) -> ExponentialBackoff {
        let growth = self.growth.max(1.0);
        let jitter = self.jitter.clamp(0.0, 1.0);
        let max = if self.max < self.initial {
            self.initial
        } else {
            self.max
        };
        let rng = self
            .seed
            .map_or_else(StdRng::from_entropy, StdRng::seed_from_u64);
        ExponentialBackoff {
            initial: self.initial,
            max,
            growth,
            jitter,
            attempts: 0,
            rng,
        }
    }
}

impl Default for ExponentialBackoffBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Exponential-backoff [`ReconnectPolicy`]. The `attempts`-th call to
/// [`Self::next_delay`] returns
/// `min(initial * growth^attempts, max) * (1 + jitter * uniform(-1, 1))`.
#[derive(Debug)]
pub struct ExponentialBackoff {
    initial: Duration,
    max: Duration,
    growth: f64,
    jitter: f64,
    attempts: u32,
    rng: StdRng,
}

impl ExponentialBackoff {
    /// Start a [`ExponentialBackoffBuilder`].
    #[must_use]
    pub const fn builder() -> ExponentialBackoffBuilder {
        ExponentialBackoffBuilder::new()
    }

    /// Construct a policy with default parameters. Convenience for code
    /// that does not need to override anything.
    #[must_use]
    pub fn new() -> Self {
        ExponentialBackoffBuilder::new().build()
    }

    /// Base delay without jitter, capped at `max`. Exposed for tests.
    #[must_use]
    pub fn base_delay_for_attempt(&self, attempt: u32) -> Duration {
        // `powf(attempt as f64)` rather than `powi(attempt as i32)` so
        // the cast is lossless (u32 → f64 fits in the mantissa).
        let scaled = self.initial.as_secs_f64() * self.growth.powf(f64::from(attempt));
        if scaled.is_finite() && scaled < self.max.as_secs_f64() {
            Duration::from_secs_f64(scaled)
        } else {
            self.max
        }
    }
}

impl Default for ExponentialBackoff {
    fn default() -> Self {
        Self::new()
    }
}

impl ReconnectPolicy for ExponentialBackoff {
    fn next_delay(&mut self) -> Duration {
        let base = self.base_delay_for_attempt(self.attempts);
        self.attempts = self.attempts.saturating_add(1);
        if self.jitter == 0.0 {
            return base;
        }
        // factor is in [1 - jitter, 1 + jitter]; jitter clamped to [0, 1]
        // means factor is in [0, 2], so this multiplication is safe.
        let factor = self.jitter.mul_add(self.rng.gen_range(-1.0..=1.0), 1.0);
        let raw = base.as_secs_f64() * factor;
        let capped = raw.min(self.max.as_secs_f64()).max(0.0);
        Duration::from_secs_f64(capped)
    }

    fn reset(&mut self) {
        self.attempts = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_builder_yields_expected_defaults() {
        let b = ExponentialBackoff::builder().build();
        assert_eq!(b.initial, Duration::from_millis(100));
        assert_eq!(b.max, Duration::from_secs(30));
        assert!((b.growth - 2.0).abs() < f64::EPSILON);
        assert!((b.jitter - 0.1).abs() < f64::EPSILON);
    }

    #[test]
    fn zero_jitter_returns_exact_base() {
        let mut b = ExponentialBackoff::builder()
            .initial(Duration::from_millis(100))
            .max(Duration::from_secs(10))
            .growth(2.0)
            .jitter(0.0)
            .build();
        assert_eq!(b.next_delay(), Duration::from_millis(100));
        assert_eq!(b.next_delay(), Duration::from_millis(200));
        assert_eq!(b.next_delay(), Duration::from_millis(400));
    }

    #[test]
    fn reset_returns_to_initial() {
        let mut b = ExponentialBackoff::builder().jitter(0.0).build();
        let initial = b.next_delay();
        for _ in 0..5 {
            let _ = b.next_delay();
        }
        b.reset();
        assert_eq!(b.next_delay(), initial);
    }
}
