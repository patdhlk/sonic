//! [`CanHealthMonitor`] — wraps `sonic_connector_core::HealthMonitor`
//! with per-interface sub-state and worst-of aggregation.
//! `REQ_0530`, `REQ_0535`.
//!
//! Aggregation rule (`REQ_0530`): the externally-visible
//! [`ConnectorHealth`] is the worst of every iface's sub-state.
//!
//! * Every iface `Up` ⇒ connector `Up`.
//! * Some iface `Degraded` (others `Up`) ⇒ connector `Degraded`.
//! * Some iface `Down` but ≥ 1 `Up` ⇒ connector `Degraded` (iface
//!   down is degraded service overall, not total loss).
//! * All ifaces `Down` ⇒ connector `Down`.
//! * Mixed `Connecting` / `Up` during bring-up ⇒ connector
//!   `Connecting` until at least one iface is `Up`.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use crossbeam_channel::{Receiver, Sender, unbounded};
use sonic_connector_core::{
    ConnectorError, ConnectorHealth, HealthEvent, HealthMonitor, IllegalTransition,
};

use crate::routing::CanIface;

/// Per-interface health discriminator. Maps onto the connector's
/// externally-visible [`ConnectorHealth`] via the worst-of aggregator
/// inside [`CanHealthMonitor::set_iface`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IfaceHealthKind {
    /// Socket open, no traffic seen yet.
    Connecting,
    /// Operational.
    Up,
    /// Error-warning / error-passive (`REQ_0632`).
    Degraded,
    /// Bus-off; awaiting reconnect (`REQ_0633`).
    Down,
}

#[derive(Debug)]
struct IfaceState {
    kind: IfaceHealthKind,
    last_error_at: Option<Instant>,
}

/// Health monitor + broadcast channel pair.
///
/// Owns the framework-level `HealthMonitor` plus a per-iface map.
/// Each per-iface update may emit a `HealthEvent` on the aggregated
/// stream — only when the worst-of aggregation transitions to a new
/// state per `ARCH_0012`.
#[derive(Debug)]
pub struct CanHealthMonitor {
    inner: Mutex<Inner>,
    tx: Sender<HealthEvent>,
    rx: Receiver<HealthEvent>,
}

#[derive(Debug)]
struct Inner {
    aggregate: HealthMonitor,
    ifaces: HashMap<CanIface, IfaceState>,
}

impl CanHealthMonitor {
    /// Construct a monitor with the configured iface set; each iface
    /// starts in `Connecting`.
    #[must_use]
    pub fn new(ifaces: &[CanIface]) -> Self {
        let mut map = HashMap::with_capacity(ifaces.len());
        for &iface in ifaces {
            map.insert(
                iface,
                IfaceState {
                    kind: IfaceHealthKind::Connecting,
                    last_error_at: None,
                },
            );
        }
        let (tx, rx) = unbounded();
        Self {
            inner: Mutex::new(Inner {
                aggregate: HealthMonitor::new(),
                ifaces: map,
            }),
            tx,
            rx,
        }
    }

    /// Snapshot the externally-visible aggregated state.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex was poisoned by a prior panic.
    pub fn current(&self) -> ConnectorHealth {
        self.inner
            .lock()
            .expect("can health monitor lock not poisoned")
            .aggregate
            .current()
            .clone()
    }

    /// Snapshot one iface's sub-state, or `None` when the iface was
    /// not registered.
    pub fn iface_kind(&self, iface: &CanIface) -> Option<IfaceHealthKind> {
        self.inner
            .lock()
            .expect("can health monitor lock not poisoned")
            .ifaces
            .get(iface)
            .map(|s| s.kind)
    }

    /// Subscriber-side receiver. Each `Clone` of the returned handle
    /// observes the same stream — `crossbeam_channel` is MPMC.
    #[must_use]
    pub fn subscribe(&self) -> Receiver<HealthEvent> {
        self.rx.clone()
    }

    /// Set one iface's sub-state and recompute the aggregate. Emits a
    /// `HealthEvent` only when the aggregate transitions.
    ///
    /// # Errors
    ///
    /// Returns [`CanHealthError::Illegal`] when the recomputed
    /// aggregate transition is illegal per `ARCH_0012` (should not
    /// happen under correct dispatcher operation, but guards against
    /// bugs).
    pub fn set_iface(
        &self,
        iface: CanIface,
        kind: IfaceHealthKind,
    ) -> Result<Option<HealthEvent>, CanHealthError> {
        let mut guard = self
            .inner
            .lock()
            .expect("can health monitor lock not poisoned");
        let entry = guard.ifaces.entry(iface).or_insert(IfaceState {
            kind,
            last_error_at: None,
        });
        entry.kind = kind;
        if matches!(kind, IfaceHealthKind::Degraded | IfaceHealthKind::Down) {
            entry.last_error_at = Some(Instant::now());
        }
        let target = aggregate(&guard.ifaces, iface);
        let current_kind = guard.aggregate.current().kind();
        if current_kind == target.kind() {
            return Ok(None);
        }
        let evt = guard
            .aggregate
            .try_transition_to(target)
            .map_err(CanHealthError::Illegal)?;
        // Broadcast — failure means no subscribers, which is fine; we
        // hold an internal Receiver so this is impossible by
        // construction.
        let _ = self.tx.send(evt.clone());
        Ok(Some(evt))
    }
}

/// Failure modes of [`CanHealthMonitor::set_iface`].
#[derive(Debug, thiserror::Error)]
pub enum CanHealthError {
    /// Aggregate transition is illegal per `ARCH_0012`.
    #[error(transparent)]
    Illegal(#[from] IllegalTransition),
}

impl From<CanHealthError> for ConnectorError {
    fn from(err: CanHealthError) -> Self {
        match err {
            CanHealthError::Illegal(e) => Self::stack(e),
        }
    }
}

/// Compute the aggregate `ConnectorHealth` from the per-iface map.
///
/// `triggering_iface` is the iface whose sub-state just changed —
/// used to populate `Degraded`'s `reason` and `Down`'s `reason` with
/// the offending iface name (`REQ_0535`'s aggregation rule).
fn aggregate(
    ifaces: &HashMap<CanIface, IfaceState>,
    triggering_iface: CanIface,
) -> ConnectorHealth {
    if ifaces.is_empty() {
        return ConnectorHealth::Connecting {
            since: Instant::now(),
        };
    }
    let mut up = 0usize;
    let mut degraded = 0usize;
    let mut down = 0usize;
    for s in ifaces.values() {
        match s.kind {
            IfaceHealthKind::Up => up += 1,
            IfaceHealthKind::Connecting => {}
            IfaceHealthKind::Degraded => degraded += 1,
            IfaceHealthKind::Down => down += 1,
        }
    }
    let total = ifaces.len();
    // All down → connector Down (REQ_0530).
    if down == total {
        return ConnectorHealth::Down {
            reason: format!("all ifaces down (latest: {triggering_iface})"),
            since: Instant::now(),
        };
    }
    // At least one Up needed before we leave Connecting.
    if up == 0 && degraded == 0 {
        // All Connecting or mix of Connecting + Down — still bringing up.
        return ConnectorHealth::Connecting {
            since: Instant::now(),
        };
    }
    // Any degraded or any down (with some up) → connector Degraded.
    if degraded > 0 || down > 0 {
        return ConnectorHealth::Degraded {
            reason: format!("iface {triggering_iface} sub-state degraded or down"),
        };
    }
    // up + connecting only, with up >= 1 → Up.
    ConnectorHealth::Up
}

#[cfg(test)]
mod tests {
    use super::*;
    use sonic_connector_core::ConnectorHealthKind;

    fn iface(name: &str) -> CanIface {
        CanIface::new(name).unwrap()
    }

    #[test]
    fn fresh_monitor_is_connecting() {
        let m = CanHealthMonitor::new(&[iface("vcan0"), iface("vcan1")]);
        assert_eq!(m.current().kind(), ConnectorHealthKind::Connecting);
    }

    #[test]
    fn worst_of_two_ifaces() {
        let a = iface("vcan0");
        let b = iface("vcan1");
        let m = CanHealthMonitor::new(&[a, b]);
        let _ = m.set_iface(a, IfaceHealthKind::Up).unwrap();
        let _ = m.set_iface(b, IfaceHealthKind::Up).unwrap();
        assert_eq!(m.current().kind(), ConnectorHealthKind::Up);

        // One iface down while the other is up → aggregate Degraded.
        let evt = m.set_iface(a, IfaceHealthKind::Down).unwrap().unwrap();
        assert_eq!(evt.to.kind(), ConnectorHealthKind::Degraded);
        assert_eq!(m.current().kind(), ConnectorHealthKind::Degraded);

        // Both down → Down.
        let evt = m.set_iface(b, IfaceHealthKind::Down).unwrap().unwrap();
        assert_eq!(evt.to.kind(), ConnectorHealthKind::Down);
    }

    #[test]
    fn subscriber_receives_aggregate_events() {
        let a = iface("vcan0");
        let m = CanHealthMonitor::new(&[a]);
        let sub = m.subscribe();
        let _ = m.set_iface(a, IfaceHealthKind::Up).unwrap();
        let evt = sub.try_recv().unwrap();
        assert_eq!(evt.to.kind(), ConnectorHealthKind::Up);
    }
}
