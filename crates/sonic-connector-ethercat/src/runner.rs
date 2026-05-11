//! [`CycleRunner`] — composes [`BusDriver`], [`CycleScheduler`],
//! [`evaluate_wkc`], and [`EthercatHealthMonitor`] into one
//! cycle-driving unit.
//!
//! The runner owns the driver by value; the health monitor is shared
//! via `Arc` so the plugin side ([`crate::EthercatConnector`])
//! observes the same transitions as the gateway side that drives
//! them.
//!
//! Tests use `MockBusDriver` to verify the runner's end-to-end
//! behaviour — bring-up transitions `Connecting → Up`, matching WKC
//! keeps the connector `Up`, mismatching WKC transitions to
//! `Degraded { reason }` with the cycle index in the reason
//! (`REQ_0319` / `REQ_0320`), and the scheduler enforces
//! skip-not-catch-up semantics (`REQ_0317`).

use std::sync::Arc;
use std::time::Instant;

use sonic_connector_core::{ConnectorError, ConnectorHealth};

use crate::driver::{BringUp, BusDriver};
use crate::health::EthercatHealthMonitor;
use crate::options::EthercatConnectorOptions;
use crate::scheduler::{CycleDecision, CycleScheduler};
use crate::wkc::{WkcVerdict, evaluate_wkc};

/// One cycle's outcome, returned from [`CycleRunner::tick`] when a
/// cycle actually fired (the scheduler reported `Fire`).
#[derive(Clone, Copy, Debug)]
pub struct CycleReport {
    /// Monotonically increasing cycle counter. Zero-indexed; the
    /// first cycle after `bring_up` is `0`.
    pub cycle_index: u64,
    /// Working counter the driver observed.
    pub working_counter: u16,
    /// WKC verdict relative to the bring-up's `expected_wkc`.
    pub verdict: WkcVerdict,
}

/// Driver-agnostic cycle runner.
#[derive(Debug)]
pub struct CycleRunner<D> {
    driver: D,
    scheduler: CycleScheduler,
    health: Arc<EthercatHealthMonitor>,
    expected_wkc: u16,
    cycle_index: u64,
}

impl<D> CycleRunner<D>
where
    D: BusDriver,
{
    /// Run `driver.bring_up()`, transition health to `Up`, and
    /// construct a runner sized to `options.cycle_time()`.
    ///
    /// # Errors
    ///
    /// Forwards any `ConnectorError` returned by `driver.bring_up()`.
    /// On error, no state changes are visible to the caller (the
    /// runner is not constructed).
    pub async fn new(
        mut driver: D,
        options: &EthercatConnectorOptions,
        health: Arc<EthercatHealthMonitor>,
    ) -> Result<Self, ConnectorError> {
        let BringUp { expected_wkc, .. } = driver.bring_up().await?;
        // Best-effort: the monitor's transition matrix only allows
        // `Up` from `Connecting` / `Degraded`. `bring_up` is a
        // start-of-life event, so the monitor is in `Connecting` by
        // construction (see `EthercatHealthMonitor::new`).
        let _ = health.transition_to(ConnectorHealth::Up);
        Ok(Self {
            driver,
            scheduler: CycleScheduler::new(options.cycle_time()),
            health,
            expected_wkc,
            cycle_index: 0,
        })
    }

    /// Cycle counter — number of cycles that have actually fired
    /// since construction (skipped polls don't count).
    #[must_use]
    pub const fn cycle_index(&self) -> u64 {
        self.cycle_index
    }

    /// Expected working counter, fixed at bring-up time.
    #[must_use]
    pub const fn expected_wkc(&self) -> u16 {
        self.expected_wkc
    }

    /// Borrow the shared health monitor.
    #[must_use]
    pub const fn health(&self) -> &Arc<EthercatHealthMonitor> {
        &self.health
    }

    /// Borrow the owned driver. Used by the gateway dispatcher (C7b)
    /// to call the trait's PDI-callback methods between cycles.
    #[must_use]
    pub const fn driver(&self) -> &D {
        &self.driver
    }

    /// Decide whether to fire a cycle at `now`. If the scheduler
    /// reports `Skip`, returns `Ok(None)` and does not call the
    /// driver. If `Fire`, calls `driver.cycle()`, evaluates the WKC,
    /// drives the health monitor, and returns the [`CycleReport`].
    ///
    /// # Errors
    ///
    /// Forwards any `ConnectorError` from `driver.cycle()`. Health
    /// transitions are best-effort — a transition that's illegal per
    /// `ARCH_0012` (e.g. the monitor is already `Down`) is silently
    /// dropped; the cycle's outcome is reported regardless.
    pub async fn tick(&mut self, now: Instant) -> Result<Option<CycleReport>, ConnectorError> {
        if self.scheduler.poll(now) == CycleDecision::Skip {
            return Ok(None);
        }
        let working_counter = self.driver.cycle().await?;
        let verdict = evaluate_wkc(self.expected_wkc, working_counter);
        self.apply_verdict(verdict);
        let report = CycleReport {
            cycle_index: self.cycle_index,
            working_counter,
            verdict,
        };
        self.cycle_index += 1;
        Ok(Some(report))
    }

    fn apply_verdict(&self, verdict: WkcVerdict) {
        match verdict {
            WkcVerdict::Match => {
                if !matches!(self.health.current(), ConnectorHealth::Up) {
                    let _ = self.health.transition_to(ConnectorHealth::Up);
                }
            }
            WkcVerdict::Mismatch { .. } => {
                if let Some(reason) = verdict.degraded_reason(self.cycle_index) {
                    let _ = self
                        .health
                        .transition_to(ConnectorHealth::Degraded { reason });
                }
            }
        }
    }
}
