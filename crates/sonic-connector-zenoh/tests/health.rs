//! Tests for `ZenohHealthMonitor` — the thin wrapper around
//! `sonic_connector_core::HealthMonitor` for the Zenoh gateway. Z1
//! lands declaration only; emit-on-session-transition is wired in Z2.

use sonic_connector_core::{ConnectorHealth, ConnectorHealthKind};
use sonic_connector_zenoh::ZenohHealthMonitor;

#[test]
fn starts_in_connecting() {
    let monitor = ZenohHealthMonitor::new();
    assert_eq!(monitor.current().kind(), ConnectorHealthKind::Connecting);
}

#[test]
fn legal_connecting_to_up_emits_one_event() {
    let monitor = ZenohHealthMonitor::new();
    let recv = monitor.subscribe();

    monitor
        .transition_to(ConnectorHealth::Up)
        .expect("connecting -> up is legal");
    assert_eq!(monitor.current().kind(), ConnectorHealthKind::Up);

    let evt = recv
        .recv_timeout(std::time::Duration::from_millis(100))
        .expect("one event emitted");
    assert_eq!(evt.to.kind(), ConnectorHealthKind::Up);
}

#[test]
fn up_to_degraded_emits_event() {
    let monitor = ZenohHealthMonitor::new();
    let recv = monitor.subscribe();

    monitor.transition_to(ConnectorHealth::Up).unwrap();
    monitor
        .transition_to(ConnectorHealth::Degraded {
            reason: "test reason".to_string(),
        })
        .expect("up -> degraded is legal");

    let _ = recv
        .recv_timeout(std::time::Duration::from_millis(100))
        .unwrap();
    let evt = recv
        .recv_timeout(std::time::Duration::from_millis(100))
        .expect("second event");
    assert_eq!(evt.to.kind(), ConnectorHealthKind::Degraded);
}
