//! Tests for the `ZenohSessionLike` trait and reply-framing helpers.
//!
//! Z1 lands only the declarative trait + frame helpers; operational
//! pub/sub + query behavior lives in `MockZenohSession` (tested in
//! `tests/mock_session.rs`).

use sonic_connector_zenoh::{FrameKind, ReplyFrame};

#[test]
fn frame_kind_byte_round_trips() {
    for k in [FrameKind::Data, FrameKind::EndOfStream, FrameKind::Timeout] {
        let byte = k.discriminator();
        let back = FrameKind::from_byte(byte).expect("known byte");
        assert_eq!(back, k, "round-trip mismatch on {k:?}");
    }
}

#[test]
fn frame_kind_byte_values_are_stable() {
    // REQ_0424 / ADR_0043 fix these byte values on the wire — do not
    // re-number without amending the spec.
    assert_eq!(FrameKind::Data.discriminator(),        0x01);
    assert_eq!(FrameKind::EndOfStream.discriminator(), 0x02);
    assert_eq!(FrameKind::Timeout.discriminator(),     0x03);
}

#[test]
fn frame_kind_rejects_unknown_byte() {
    assert!(FrameKind::from_byte(0x00).is_none());
    assert!(FrameKind::from_byte(0x04).is_none());
    assert!(FrameKind::from_byte(0xFF).is_none());
}

#[test]
fn reply_frame_encode_data_chunk() {
    let body = [0xAAu8, 0xBB, 0xCC, 0xDD];
    let mut buf = [0u8; 16];
    let len = ReplyFrame::encode_data(&body, &mut buf).expect("fits");
    assert_eq!(len, 5);
    assert_eq!(buf[0], 0x01);
    assert_eq!(&buf[1..5], &body);
}

#[test]
fn reply_frame_encode_end_of_stream_is_one_byte() {
    let mut buf = [0u8; 4];
    let len = ReplyFrame::encode_end_of_stream(&mut buf).expect("fits");
    assert_eq!(len, 1);
    assert_eq!(buf[0], 0x02);
}

#[test]
fn reply_frame_encode_timeout_is_one_byte() {
    let mut buf = [0u8; 4];
    let len = ReplyFrame::encode_timeout(&mut buf).expect("fits");
    assert_eq!(len, 1);
    assert_eq!(buf[0], 0x03);
}

#[test]
fn reply_frame_encode_data_rejects_undersized_buffer() {
    let body = [0xAAu8, 0xBB, 0xCC, 0xDD];
    let mut buf = [0u8; 3];
    assert!(ReplyFrame::encode_data(&body, &mut buf).is_err());
}

#[test]
fn reply_frame_decode_data_chunk() {
    let envelope = [0x01u8, 0xAA, 0xBB, 0xCC];
    let frame = ReplyFrame::decode(&envelope).expect("known kind");
    assert_eq!(frame.kind(), FrameKind::Data);
    assert_eq!(frame.body(), &[0xAA, 0xBB, 0xCC]);
}

#[test]
fn reply_frame_decode_end_of_stream() {
    let envelope = [0x02u8];
    let frame = ReplyFrame::decode(&envelope).expect("known kind");
    assert_eq!(frame.kind(), FrameKind::EndOfStream);
    assert!(frame.body().is_empty());
}

#[test]
fn reply_frame_decode_rejects_empty_envelope() {
    let envelope: [u8; 0] = [];
    assert!(ReplyFrame::decode(&envelope).is_err());
}

#[test]
fn reply_frame_decode_rejects_unknown_kind() {
    let envelope = [0x09u8, 0xAA];
    assert!(ReplyFrame::decode(&envelope).is_err());
}

/// `ZenohSessionLike` should be object-unsafe-friendly enough that we
/// can monomorphise over it. We just compile-check the bounds.
#[test]
fn zenoh_session_like_is_send_sync_static() {
    use sonic_connector_zenoh::ZenohSessionLike;
    #[allow(dead_code)]
    fn assert_bounds<S: ZenohSessionLike + Send + Sync + 'static>() {}
    fn _use<S: ZenohSessionLike + Send + Sync + 'static>() {
        assert_bounds::<S>();
    }
}

/// `SessionState` should be a simple enum so health-monitor wiring can
/// match on it.
#[test]
fn session_state_variants_compile() {
    use sonic_connector_zenoh::SessionState;
    let _ = SessionState::Connecting;
    let _ = SessionState::Alive;
    let _ = SessionState::Closed { reason: "test".into() };
}
