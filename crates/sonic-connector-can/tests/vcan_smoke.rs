//! `TEST_0512` — Linux raw-socket smoke against `vcan0`.
//! `REQ_0502`, `REQ_0613`, `REQ_0614`.
//!
//! Exercises [`crate::RealCanInterface`] end-to-end against the
//! Linux kernel's virtual CAN driver. Two separate
//! `RealCanInterface` instances bound to the same iface — one
//! sends classical frames, the other receives them — verify that
//! the kernel `PF_CAN` socket family round-trips frames correctly.
//!
//! ## CI pre-requisites
//!
//! The kernel `vcan` module must be loaded and `vcan0` must exist:
//!
//! ```sh
//! sudo modprobe vcan
//! sudo ip link add dev vcan0 type vcan
//! sudo ip link set up vcan0
//! ```
//!
//! The test is marked `#[ignore]` so plain `cargo test --features
//! socketcan-integration` skips it. CI runs it explicitly with
//! `--include-ignored` once the vcan setup script succeeds.

#![cfg(all(feature = "socketcan-integration", target_os = "linux"))]
#![allow(clippy::doc_markdown, clippy::cast_possible_truncation)]

use std::time::Duration;

use sonic_connector_can::{
    CanData, CanFdFlags, CanFrame, CanFrameKind, CanId, CanIface, CanInterfaceLike,
    RealCanInterface,
};

#[tokio::test]
#[ignore = "requires Linux vcan kernel module and `vcan0` interface"]
async fn test_0512_classical_round_trip_via_vcan0() {
    let iface = CanIface::new("vcan0").expect("vcan0 fits IFNAMSIZ");

    // Two sockets bound to the same iface — both see frames written
    // by either side because PF_CAN raw sockets are broadcast.
    let mut tx = RealCanInterface::open(iface).expect("open tx socket on vcan0");
    let mut rx = RealCanInterface::open(iface).expect("open rx socket on vcan0");

    // Use accept-all filter on the rx side so all frames reach it
    // regardless of ID.
    rx.apply_filter(&[sonic_connector_can::CanFilter {
        can_id: 0,
        can_mask: 0,
    }])
    .expect("apply accept-all filter");

    // Send 10 classical frames; assert each round-trips.
    for i in 0u8..10 {
        let data = CanData::new(
            CanId::standard(0x100 + u16::from(i)).expect("id < 0x800"),
            CanFrameKind::Classical,
            CanFdFlags::empty(),
            &[i, i.wrapping_add(1), 0xAA, 0xBB],
        )
        .expect("classical frame fits");
        tx.send_classical(&data).await.expect("send_classical");

        // Bound the await so a missing vcan does not hang the test.
        let frame = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("recv within 2s")
            .expect("recv ok");
        match frame {
            CanFrame::Data(d) => {
                assert_eq!(d.id.value, u32::from(0x100 + u16::from(i)));
                assert!(!d.id.extended);
                assert_eq!(d.payload(), data.payload());
            }
            other => panic!("expected Data frame, got {other:?}"),
        }
    }
}
