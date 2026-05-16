//! Tests for the `ZenohRouting` struct and `KeyExprOwned` validation.
//!
//! Verifies `REQ_0401` — routing carries `key_expr` + pub/sub `QoS` and rejects
//! invalid key expressions on the plugin side before any IPC service is
//! created.

use sonic_connector_core::Routing;
use sonic_connector_zenoh::{CongestionControl, KeyExprOwned, Priority, Reliability, ZenohRouting};

#[test]
fn key_expr_round_trips_valid_strings() {
    let k = KeyExprOwned::try_from("robot/arm/joint1").expect("valid key");
    assert_eq!(k.as_str(), "robot/arm/joint1");
}

#[test]
fn key_expr_rejects_empty_string() {
    let err = KeyExprOwned::try_from("").expect_err("empty rejected");
    let msg = err.to_string();
    assert!(msg.contains("empty"), "error mentions emptiness: {msg}");
}

#[test]
fn key_expr_rejects_leading_slash() {
    let err = KeyExprOwned::try_from("/robot/arm").expect_err("leading slash rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("leading"),
        "error mentions leading-slash: {msg}"
    );
}

#[test]
fn key_expr_rejects_trailing_slash() {
    let err = KeyExprOwned::try_from("robot/arm/").expect_err("trailing slash rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("trailing"),
        "error mentions trailing-slash: {msg}"
    );
}

#[test]
fn key_expr_rejects_double_slash() {
    let err = KeyExprOwned::try_from("robot//arm").expect_err("double slash rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("empty chunk"),
        "error mentions empty chunk: {msg}"
    );
}

#[test]
fn zenoh_routing_carries_all_fields() {
    let routing = ZenohRouting::new(KeyExprOwned::try_from("robot/arm/joint1").unwrap())
        .with_congestion_control(CongestionControl::Block)
        .with_priority(Priority::RealTime)
        .with_reliability(Reliability::Reliable)
        .with_express(true);

    assert_eq!(routing.key_expr().as_str(), "robot/arm/joint1");
    assert_eq!(routing.congestion_control(), CongestionControl::Block);
    assert_eq!(routing.priority(), Priority::RealTime);
    assert_eq!(routing.reliability(), Reliability::Reliable);
    assert!(routing.express());
}

#[test]
fn zenoh_routing_defaults_are_safe() {
    let routing = ZenohRouting::new(KeyExprOwned::try_from("robot/arm/joint1").unwrap());
    // Default to Drop (don't block sender on congestion) + best-effort
    // delivery + non-express (allow batching). Real-time priority is opt-in.
    assert_eq!(routing.congestion_control(), CongestionControl::Drop);
    assert_eq!(routing.priority(), Priority::Data);
    assert_eq!(routing.reliability(), Reliability::BestEffort);
    assert!(!routing.express());
}

/// `ZenohRouting` must implement the framework's `Routing` marker trait so
/// it can be plugged into `ChannelDescriptor<R: Routing, N>`.
#[test]
fn zenoh_routing_implements_routing_marker() {
    fn assert_routing<R: Routing>() {}
    assert_routing::<ZenohRouting>();
}
