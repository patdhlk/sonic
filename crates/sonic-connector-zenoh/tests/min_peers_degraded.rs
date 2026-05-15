//! Verify `min_peers` drives `ConnectorHealth::Degraded` transitions
//! when the session reports a peer count below the configured floor.
//! Asserts one `HealthEvent` per observation change (`REQ_0442`).
//!
//! Mirrors the subscription pattern of `health_transitions.rs` —
//! `HealthSubscription::try_next()` rather than the plan-template's
//! `try_recv()`.

use std::sync::Arc;
use std::time::Duration;

use sonic_connector_codec::JsonCodec;
use sonic_connector_core::HealthEvent;
use sonic_connector_host::{Connector, HealthSubscription};
use sonic_connector_zenoh::session::SessionState;
use sonic_connector_zenoh::{
    MockZenohSession, ZenohConnector, ZenohConnectorOptions, ZenohState,
};
use sonic_executor::Executor;

/// Drain one health event with a deadline.
fn next_event(sub: &HealthSubscription, deadline: Duration) -> Option<HealthEvent> {
    let stop = std::time::Instant::now() + deadline;
    while std::time::Instant::now() < stop {
        match sub.try_next() {
            Ok(Some(ev)) => return Some(ev),
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(e) => panic!("subscription disconnected: {e}"),
        }
    }
    None
}

#[test]
fn min_peers_drives_degraded_transitions() {
    let session = Arc::new(MockZenohSession::new());
    // Start below the floor: peer_count == 0, min_peers == 2.
    session.set_peer_count(0);

    let opts = ZenohConnectorOptions::builder()
        .dispatcher_tick(Duration::from_millis(1))
        .tokio_worker_threads(1)
        .min_peers(2)
        .build();
    let state = Arc::new(ZenohState::new(opts));
    let mut connector =
        ZenohConnector::new(state, Arc::clone(&session), JsonCodec).unwrap();

    let subscription: HealthSubscription = connector.subscribe_health();
    let mut exec = Executor::builder().worker_threads(0).build().unwrap();
    connector.register_with(&mut exec).unwrap();

    // The watcher emits an initial event reflecting the starting
    // observation. Mock starts in `SessionState::Alive`, so the tuple
    // `(Alive, peer_count=0, min_peers=Some(2))` maps to `Degraded`.
    let first = next_event(&subscription, Duration::from_millis(500))
        .expect("initial Degraded event");
    assert!(
        format!("{first:?}").contains("Degraded"),
        "initial event should be Degraded; got {first:?}"
    );

    // Cross the floor upward: 0 -> 2 peers should yield Up.
    session.set_peer_count(2);
    let up_event = next_event(&subscription, Duration::from_millis(500))
        .expect("Degraded -> Up event");
    assert!(
        format!("{up_event:?}").contains("Up"),
        "expected Up; got {up_event:?}"
    );

    // Cross the floor downward: 2 -> 1 peers should yield Degraded.
    session.set_peer_count(1);
    let degraded_event = next_event(&subscription, Duration::from_millis(500))
        .expect("Up -> Degraded event");
    assert!(
        format!("{degraded_event:?}").contains("Degraded"),
        "expected Degraded; got {degraded_event:?}"
    );

    // Close the session: Degraded -> Down.
    session.set_state(SessionState::Closed {
        reason: "test".into(),
    });
    let down_event = next_event(&subscription, Duration::from_millis(500))
        .expect("Degraded -> Down event");
    assert!(
        format!("{down_event:?}").contains("Down"),
        "expected Down; got {down_event:?}"
    );

    // No further events expected.
    assert!(
        next_event(&subscription, Duration::from_millis(200)).is_none(),
        "no further health events after Down"
    );

    connector.stop_dispatcher();
}
