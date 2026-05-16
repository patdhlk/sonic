//! Tests for the bounded outbound and inbound bridges.
//!
//! Verifies `REQ_0404` (bounded), `REQ_0405` (outbound `BackPressure`),
//! `REQ_0406` (inbound `DroppedInbound` + running count).

use sonic_connector_zenoh::{InboundBridge, InboundOutcome, OutboundBridge, OutboundError};

#[test]
fn outbound_bridge_enforces_capacity() {
    let bridge: OutboundBridge<u32> = OutboundBridge::new(2);
    assert_eq!(bridge.capacity(), 2);
    bridge.try_send(1).expect("first slot");
    bridge.try_send(2).expect("second slot");

    let err = bridge.try_send(3).expect_err("third over capacity");
    match err {
        OutboundError::BackPressure(3) => {}
        other => panic!("expected BackPressure(3), got {other:?}"),
    }
}

#[test]
fn outbound_bridge_drains_into_recv() {
    let bridge: OutboundBridge<u32> = OutboundBridge::new(4);
    bridge.try_send(10).unwrap();
    bridge.try_send(20).unwrap();
    assert_eq!(bridge.try_recv(), Some(10));
    assert_eq!(bridge.try_recv(), Some(20));
    assert_eq!(bridge.try_recv(), None);
}

#[test]
fn outbound_bridge_returns_message_on_backpressure() {
    let bridge: OutboundBridge<String> = OutboundBridge::new(1);
    bridge.try_send("first".into()).unwrap();

    let err = bridge
        .try_send("dropped".into())
        .expect_err("over capacity");
    let recovered = err.into_inner();
    assert_eq!(recovered, "dropped");
}

#[test]
fn inbound_bridge_increments_dropped_count_on_saturation() {
    let bridge: InboundBridge<u32> = InboundBridge::new(1);
    assert!(matches!(bridge.try_send(1), InboundOutcome::Sent));

    let out = bridge.try_send(2);
    assert!(matches!(out, InboundOutcome::Dropped { count: 1 }));
    let out = bridge.try_send(3);
    assert!(matches!(out, InboundOutcome::Dropped { count: 2 }));
    assert_eq!(bridge.dropped_count(), 2);

    assert_eq!(bridge.try_recv(), Some(1));
    assert_eq!(bridge.try_recv(), None);
}

#[test]
fn capacity_zero_clamps_to_one() {
    let outbound: OutboundBridge<u32> = OutboundBridge::new(0);
    assert_eq!(outbound.capacity(), 1);
    let inbound: InboundBridge<u32> = InboundBridge::new(0);
    assert_eq!(inbound.capacity(), 1);
}
