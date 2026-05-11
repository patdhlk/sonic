//! TEST_0101 — `ConnectorHealth` state machine transitions per
//! `ARCH_0012`. Asserts that every legal transition emits exactly one
//! `HealthEvent` whose `from` / `to` match, and that illegal transitions
//! panic (`transition_to`) or return `Err` (`try_transition_to`).

#![allow(clippy::doc_markdown)]

use std::time::Instant;

use sonic_connector_core::{
    ConnectorHealth, ConnectorHealthKind, HealthMonitor, IllegalTransition,
};

/// Construct a representative variant for each discriminator. The
/// monitor's transition matrix uses discriminants only, so the specific
/// field values do not matter — but using realistic values keeps the
/// test readable.
fn sample(kind: ConnectorHealthKind) -> ConnectorHealth {
    match kind {
        ConnectorHealthKind::Up => ConnectorHealth::Up,
        ConnectorHealthKind::Connecting => ConnectorHealth::Connecting {
            since: Instant::now(),
        },
        ConnectorHealthKind::Degraded => ConnectorHealth::Degraded {
            reason: "test".into(),
        },
        ConnectorHealthKind::Down => ConnectorHealth::Down {
            reason: "test".into(),
            since: Instant::now(),
        },
    }
}

/// Set the monitor's current state by walking through legal transitions
/// from the initial `Connecting`. Used by tests that need to start from
/// a non-`Connecting` discriminator.
fn monitor_at(kind: ConnectorHealthKind) -> HealthMonitor {
    let mut m = HealthMonitor::new();
    match kind {
        ConnectorHealthKind::Connecting => m,
        ConnectorHealthKind::Up => {
            m.transition_to(sample(ConnectorHealthKind::Up));
            m
        }
        ConnectorHealthKind::Degraded => {
            m.transition_to(sample(ConnectorHealthKind::Up));
            m.transition_to(sample(ConnectorHealthKind::Degraded));
            m
        }
        ConnectorHealthKind::Down => {
            m.transition_to(sample(ConnectorHealthKind::Down));
            m
        }
    }
}

const ALL_KINDS: [ConnectorHealthKind; 4] = [
    ConnectorHealthKind::Up,
    ConnectorHealthKind::Connecting,
    ConnectorHealthKind::Degraded,
    ConnectorHealthKind::Down,
];

/// Legal edges per `ARCH_0012` (encoded as `(from, to)` pairs).
const LEGAL_EDGES: [(ConnectorHealthKind, ConnectorHealthKind); 7] = [
    (ConnectorHealthKind::Connecting, ConnectorHealthKind::Up),
    (ConnectorHealthKind::Connecting, ConnectorHealthKind::Down),
    (ConnectorHealthKind::Up, ConnectorHealthKind::Degraded),
    (ConnectorHealthKind::Up, ConnectorHealthKind::Down),
    (ConnectorHealthKind::Degraded, ConnectorHealthKind::Up),
    (ConnectorHealthKind::Degraded, ConnectorHealthKind::Down),
    (ConnectorHealthKind::Down, ConnectorHealthKind::Connecting),
];

/// Every legal transition emits exactly one `HealthEvent` with the
/// expected `from` / `to` discriminators, and the monitor's current
/// state becomes the target.
#[test]
fn every_legal_transition_emits_one_event() {
    for (from, to) in LEGAL_EDGES {
        let mut m = monitor_at(from);
        let pre_at = Instant::now();
        let ev = m.transition_to(sample(to));
        assert_eq!(
            ev.from.kind(),
            from,
            "event.from mismatch on {from:?}→{to:?}"
        );
        assert_eq!(ev.to.kind(), to, "event.to mismatch on {from:?}→{to:?}");
        assert!(ev.at >= pre_at, "event.at not monotonic on {from:?}→{to:?}");
        assert_eq!(
            m.current().kind(),
            to,
            "monitor.current() not updated on {from:?}→{to:?}"
        );
    }
}

/// `try_transition_to` returns `Err(IllegalTransition)` for every pair
/// not listed in `LEGAL_EDGES`, and the monitor's state is **not**
/// changed on failure.
#[test]
fn try_transition_rejects_every_illegal_pair_without_mutating_state() {
    for from in ALL_KINDS {
        for to in ALL_KINDS {
            if LEGAL_EDGES.contains(&(from, to)) {
                continue;
            }
            let mut m = monitor_at(from);
            let before = m.current().kind();
            let err = m
                .try_transition_to(sample(to))
                .expect_err(&format!("expected IllegalTransition on {from:?}→{to:?}"));
            let IllegalTransition { from: f, to: t } = err;
            assert_eq!(f, from);
            assert_eq!(t, to);
            assert_eq!(
                m.current().kind(),
                before,
                "monitor state mutated after illegal {from:?}→{to:?}"
            );
        }
    }
}

/// `transition_to` (the panic-on-illegal variant) panics on a
/// representative illegal edge — `Up → Connecting` skips the required
/// `Down` state. Single representative pair chosen so the test name
/// names a concrete case.
#[test]
#[should_panic(expected = "illegal health transition")]
fn transition_to_panics_on_illegal_up_to_connecting() {
    let mut m = monitor_at(ConnectorHealthKind::Up);
    let _ = m.transition_to(sample(ConnectorHealthKind::Connecting));
}

/// Same-discriminant transitions are illegal — exercised on every
/// variant so a future spec change that allows e.g. `Down → Down` for
/// updated reason has to flip this test deliberately.
#[test]
fn same_discriminant_transitions_are_illegal() {
    for kind in ALL_KINDS {
        let mut m = monitor_at(kind);
        let err = m
            .try_transition_to(sample(kind))
            .expect_err(&format!("{kind:?} → {kind:?} should be illegal"));
        assert_eq!(err.from, kind);
        assert_eq!(err.to, kind);
    }
}
