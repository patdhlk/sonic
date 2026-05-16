//! Verify the gateway emits a `HealthEvent` for every observed
//! transition of the underlying session state (`REQ_0442`).
//!
//! Z4e wires a polling watcher task in `register_with` that reads
//! `session.state()` every 100ms and broadcasts a `HealthEvent` to
//! subscribers whenever the discriminant changes.

use std::sync::Arc;
use std::time::Duration;

use sonic_connector_codec::JsonCodec;
use sonic_connector_host::{Connector, HealthSubscription};
use sonic_connector_zenoh::session::SessionState;
use sonic_connector_zenoh::{MockZenohSession, ZenohConnector, ZenohConnectorOptions, ZenohState};
use sonic_executor::Executor;

#[test]
fn health_event_emitted_on_session_close() {
    let session = Arc::new(MockZenohSession::new());
    let opts = ZenohConnectorOptions::builder()
        .dispatcher_tick(Duration::from_millis(1))
        .tokio_worker_threads(1)
        .build();
    let state = Arc::new(ZenohState::new(opts));
    let mut connector = ZenohConnector::new(state, Arc::clone(&session), JsonCodec).unwrap();

    let subscription: HealthSubscription = connector.subscribe_health();
    let mut exec = Executor::builder().worker_threads(0).build().unwrap();
    connector.register_with(&mut exec).unwrap();

    // The watcher's first tick observes the mock's initial `Alive` state
    // and drives the monitor `Connecting -> Up`. Wait for that event so
    // the subsequent `set_state(Closed)` produces a clean `Up -> Down`.
    let deadline_up = std::time::Instant::now() + Duration::from_millis(500);
    let mut saw_up = false;
    while !saw_up && std::time::Instant::now() < deadline_up {
        match subscription.try_next() {
            Ok(Some(_)) => saw_up = true,
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => panic!("subscription disconnected before initial event: {e}"),
        }
    }
    assert!(
        saw_up,
        "expected initial HealthEvent emitted by the watcher after register_with"
    );

    // Force a transition: alive -> closed.
    session.set_state(SessionState::Closed {
        reason: "test-induced".into(),
    });

    // Within ~500ms (5 watcher ticks of 100ms) we should observe one
    // event for the new transition.
    let deadline = std::time::Instant::now() + Duration::from_millis(500);
    let mut saw_event = false;
    while !saw_event && std::time::Instant::now() < deadline {
        match subscription.try_next() {
            Ok(Some(_event)) => saw_event = true,
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => panic!("subscription disconnected: {e}"),
        }
    }
    assert!(
        saw_event,
        "expected at least one HealthEvent after session closed"
    );

    connector.stop_dispatcher();
}
