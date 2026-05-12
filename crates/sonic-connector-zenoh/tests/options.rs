//! Tests for the `ZenohConnectorOptions` typed builder.
//!
//! Verifies `REQ_0440` (`SessionMode` default), `REQ_0443` (locators surfaced
//! verbatim), parts of `REQ_0425` (query defaults), and `REQ_0404` (bridge
//! capacities configurable).

use std::time::Duration;

use sonic_connector_zenoh::{
    Consolidation, Locator, QueryTarget, SessionMode, ZenohConnectorOptions,
};

#[test]
fn defaults_are_peer_mode() {
    let opts = ZenohConnectorOptions::builder().build();
    assert_eq!(opts.mode, SessionMode::Peer);
    assert!(opts.connect.is_empty());
    assert!(opts.listen.is_empty());
}

#[test]
fn default_query_target_is_all() {
    let opts = ZenohConnectorOptions::builder().build();
    assert_eq!(opts.query_target, QueryTarget::All);
    assert_eq!(opts.query_consolidation, Consolidation::None);
    assert!(opts.query_timeout >= Duration::from_millis(1));
}

#[test]
fn default_bridge_capacities_are_nonzero() {
    let opts = ZenohConnectorOptions::builder().build();
    assert!(opts.outbound_bridge_capacity >= 1);
    assert!(opts.inbound_bridge_capacity >= 1);
}

#[test]
fn builder_overrides_session_mode() {
    let opts = ZenohConnectorOptions::builder()
        .mode(SessionMode::Client)
        .build();
    assert_eq!(opts.mode, SessionMode::Client);
}

#[test]
fn builder_appends_connect_locators_in_order() {
    let opts = ZenohConnectorOptions::builder()
        .connect(Locator::new("tcp/192.168.1.1:7447"))
        .connect(Locator::new("tcp/192.168.1.2:7447"))
        .build();
    assert_eq!(opts.connect.len(), 2);
    assert_eq!(opts.connect[0].as_str(), "tcp/192.168.1.1:7447");
    assert_eq!(opts.connect[1].as_str(), "tcp/192.168.1.2:7447");
}

#[test]
fn builder_overrides_query_timeout() {
    let opts = ZenohConnectorOptions::builder()
        .query_timeout(Duration::from_millis(250))
        .build();
    assert_eq!(opts.query_timeout, Duration::from_millis(250));
}

#[test]
fn builder_overrides_bridge_capacities() {
    let opts = ZenohConnectorOptions::builder()
        .outbound_bridge_capacity(8)
        .inbound_bridge_capacity(16)
        .build();
    assert_eq!(opts.outbound_bridge_capacity, 8);
    assert_eq!(opts.inbound_bridge_capacity, 16);
}

#[test]
fn min_peers_defaults_none() {
    let opts = ZenohConnectorOptions::builder().build();
    assert_eq!(opts.min_peers, None);
}

#[test]
fn builder_sets_min_peers() {
    let opts = ZenohConnectorOptions::builder().min_peers(2).build();
    assert_eq!(opts.min_peers, Some(2));
}

#[test]
fn default_tokio_worker_threads_is_one() {
    let opts = ZenohConnectorOptions::builder().build();
    assert_eq!(opts.tokio_worker_threads, 1);
}

#[test]
fn builder_overrides_tokio_worker_threads() {
    let opts = ZenohConnectorOptions::builder()
        .tokio_worker_threads(4)
        .build();
    assert_eq!(opts.tokio_worker_threads, 4);
}

#[test]
fn tokio_worker_threads_zero_clamps_to_one() {
    let opts = ZenohConnectorOptions::builder()
        .tokio_worker_threads(0)
        .build();
    assert_eq!(opts.tokio_worker_threads, 1);
}

#[test]
fn default_dispatcher_tick_is_one_millisecond() {
    let opts = ZenohConnectorOptions::builder().build();
    assert_eq!(opts.dispatcher_tick, std::time::Duration::from_millis(1));
}

#[test]
fn builder_overrides_dispatcher_tick() {
    let opts = ZenohConnectorOptions::builder()
        .dispatcher_tick(std::time::Duration::from_millis(5))
        .build();
    assert_eq!(opts.dispatcher_tick, std::time::Duration::from_millis(5));
}
