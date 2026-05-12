//! Saturation tests covering `REQ_0405` (`BackPressure` on outbound)
//! and `REQ_0406` (`DroppedInbound` on inbound).
//!
//! These tests verify the bridge-level contract — the underlying
//! `OutboundBridge` / `InboundBridge` types from `crate::bridge`. The
//! end-to-end plugin → gateway → session → plugin pipeline can also
//! saturate (under the right configuration), but iceoryx2's internal
//! queue depth makes that path non-deterministic without bespoke
//! pacing. The bridge-level test pins the contract; integration in
//! later stages (real session in Z4) may add an additional end-to-end
//! saturation test once iceoryx2's queue semantics are explicit.

use sonic_connector_zenoh::{InboundBridge, InboundOutcome, OutboundBridge, OutboundError};

#[test]
fn outbound_bridge_full_returns_backpressure_with_payload() {
    // REQ_0405: when the outbound bridge is full, `try_send` returns
    // `BackPressure(T)` carrying the rejected payload.
    let bridge: OutboundBridge<u32> = OutboundBridge::new(2);
    bridge.try_send(1).expect("first slot");
    bridge.try_send(2).expect("second slot");

    let err = bridge.try_send(99).expect_err("over capacity");
    match err {
        OutboundError::BackPressure(99) => {}
        other => panic!("expected BackPressure(99), got {other:?}"),
    }
}

#[test]
fn outbound_bridge_drains_then_recovers() {
    // REQ_0405 follow-up: after draining, the bridge accepts again.
    let bridge: OutboundBridge<u32> = OutboundBridge::new(1);
    bridge.try_send(10).unwrap();
    assert!(bridge.try_send(20).is_err()); // BackPressure
    bridge.try_recv().expect("drained 10");
    bridge.try_send(20).expect("now accepts");
}

#[test]
fn inbound_bridge_full_records_drop_count() {
    // REQ_0406: when the inbound bridge is full, `try_send` returns
    // `Dropped { count }` reflecting the running drop count.
    let bridge: InboundBridge<u32> = InboundBridge::new(1);
    assert!(matches!(bridge.try_send(1), InboundOutcome::Sent));
    assert!(matches!(
        bridge.try_send(2),
        InboundOutcome::Dropped { count: 1 }
    ));
    assert!(matches!(
        bridge.try_send(3),
        InboundOutcome::Dropped { count: 2 }
    ));
    assert!(matches!(
        bridge.try_send(4),
        InboundOutcome::Dropped { count: 3 }
    ));
    assert_eq!(bridge.dropped_count(), 3);
}

#[test]
fn inbound_bridge_drop_count_persists_after_drain() {
    // REQ_0406 detail: drop count is cumulative across drains.
    let bridge: InboundBridge<u32> = InboundBridge::new(1);
    assert!(matches!(bridge.try_send(1), InboundOutcome::Sent));
    assert!(matches!(
        bridge.try_send(2),
        InboundOutcome::Dropped { count: 1 }
    ));
    bridge.try_recv(); // drain
    assert!(matches!(bridge.try_send(3), InboundOutcome::Sent));
    assert!(matches!(
        bridge.try_send(4),
        InboundOutcome::Dropped { count: 2 }
    ));
    assert_eq!(bridge.dropped_count(), 2);
}
