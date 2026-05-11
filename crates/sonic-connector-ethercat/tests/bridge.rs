//! TEST_0212 (bridge bounded capacity), TEST_0213 (outbound bridge
//! saturation surfaces as BackPressure), TEST_0214 (inbound bridge
//! saturation surfaces as DroppedInbound with running count).

#![allow(clippy::doc_markdown)]

use sonic_connector_ethercat::{InboundBridge, InboundOutcome, OutboundBridge, OutboundError};

#[test]
fn outbound_capacity_matches_constructor() {
    let b = OutboundBridge::<u32>::new(4);
    assert_eq!(b.capacity(), 4);
}

#[test]
fn outbound_accepts_up_to_capacity_then_back_pressure() {
    let b = OutboundBridge::<u32>::new(3);
    // Three accepted ...
    b.try_send(1).expect("first");
    b.try_send(2).expect("second");
    b.try_send(3).expect("third");
    // ... the fourth fails with BackPressure carrying the original.
    let err = b.try_send(99).expect_err("fourth must fail");
    match err {
        OutboundError::BackPressure(v) => assert_eq!(v, 99),
        OutboundError::Disconnected(_) => panic!("not expected — receiver still alive"),
    }
}

#[test]
fn outbound_recovers_after_drain() {
    let b = OutboundBridge::<u32>::new(2);
    b.try_send(10).unwrap();
    b.try_send(20).unwrap();
    assert!(b.try_send(30).is_err());
    // Drain one — capacity opens up.
    assert_eq!(b.try_recv(), Some(10));
    b.try_send(30).expect("space available after drain");
}

#[test]
fn inbound_capacity_matches_constructor() {
    let b = InboundBridge::<u32>::new(8);
    assert_eq!(b.capacity(), 8);
    assert_eq!(b.dropped_count(), 0);
}

#[test]
fn inbound_drops_past_capacity_and_counts() {
    let b = InboundBridge::<u32>::new(2);
    assert!(matches!(b.try_send(1), InboundOutcome::Sent));
    assert!(matches!(b.try_send(2), InboundOutcome::Sent));
    // Third drops, count = 1.
    let out = b.try_send(3);
    match out {
        InboundOutcome::Dropped { count } => assert_eq!(count, 1),
        InboundOutcome::Sent => panic!("third should have dropped"),
    }
    // Fourth drops, count = 2.
    let out = b.try_send(4);
    match out {
        InboundOutcome::Dropped { count } => assert_eq!(count, 2),
        InboundOutcome::Sent => panic!("fourth should have dropped"),
    }
    assert_eq!(b.dropped_count(), 2);
}

#[test]
fn inbound_recovers_after_drain() {
    let b = InboundBridge::<u32>::new(2);
    b.try_send(1);
    b.try_send(2);
    // Two in flight; third drops.
    assert!(matches!(b.try_send(3), InboundOutcome::Dropped { .. }));
    // Drain one — capacity opens up for one more.
    assert_eq!(b.try_recv(), Some(1));
    assert!(matches!(b.try_send(4), InboundOutcome::Sent));
    assert_eq!(b.dropped_count(), 1);
}
