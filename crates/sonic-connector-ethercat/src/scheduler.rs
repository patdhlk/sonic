//! [`CycleScheduler`] — cycle-time pacing with skip-not-catch-up
//! semantics. `REQ_0317`.
//!
//! Pure logic over an external clock — the caller passes the current
//! [`Instant`] on every poll. Tests use a synthetic clock to verify
//! that arbitrary scheduling jitter never causes more than one tick
//! to fire when multiple cycle periods have elapsed since the last
//! tick.
//!
//! Production code feeds this with `tokio::time::Instant::now()` or
//! `std::time::Instant::now()` from inside the gateway's cycle loop.
//! The tokio integration (`tokio::time::interval` with
//! `MissedTickBehavior::Skip`) and this scheduler agree on
//! semantics; we keep our own implementation so the logic can be
//! verified deterministically.

use std::time::{Duration, Instant};

/// Cycle-time pacing decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CycleDecision {
    /// A cycle should fire now. The scheduler has updated its
    /// internal state to record this tick.
    Fire,
    /// It is too soon since the last tick. Skip this poll and try
    /// again later.
    Skip,
}

/// Pure-logic cycle scheduler with `REQ_0317` skip-not-catch-up
/// semantics.
#[derive(Clone, Copy, Debug)]
pub struct CycleScheduler {
    interval: Duration,
    last_tick: Option<Instant>,
}

impl CycleScheduler {
    /// Construct a scheduler with the given cycle period.
    #[must_use]
    pub const fn new(interval: Duration) -> Self {
        Self {
            interval,
            last_tick: None,
        }
    }

    /// Configured cycle period.
    #[must_use]
    pub const fn interval(&self) -> Duration {
        self.interval
    }

    /// Decide whether to fire a cycle at `now`.
    ///
    /// The first call always fires (`last_tick == None`). Subsequent
    /// calls fire iff `now - last_tick >= interval`. If many
    /// intervals have elapsed since the last tick — e.g. the
    /// scheduler was paused while another thread held the CPU —
    /// **only one** fire is reported here; the missed cycles are
    /// silently skipped (`REQ_0317`).
    pub fn poll(&mut self, now: Instant) -> CycleDecision {
        let should_fire = match self.last_tick {
            None => true,
            Some(prev) => now.duration_since(prev) >= self.interval,
        };
        if should_fire {
            self.last_tick = Some(now);
            CycleDecision::Fire
        } else {
            CycleDecision::Skip
        }
    }

    /// Last instant at which a cycle fired, or `None` if no cycle
    /// has fired yet.
    #[must_use]
    pub const fn last_tick(&self) -> Option<Instant> {
        self.last_tick
    }
}
