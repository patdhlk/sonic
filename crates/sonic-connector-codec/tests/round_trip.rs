//! TEST_0110 — `JsonCodec` round-trip property test. Verifies that for
//! every input the codec accepts, `encode` followed by `decode` yields
//! a value equal to the original (`REQ_0210`, `REQ_0212`).

#![cfg(feature = "json")]
#![allow(clippy::doc_markdown)]

use proptest::prelude::*;
use serde::{Deserialize, Serialize};
use sonic_connector_codec::{JsonCodec, PayloadCodec};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Msg {
    value: i64,
    name: String,
    flags: Vec<bool>,
}

proptest! {
    /// Property: `decode(encode(v)) == v` for representative `Msg`
    /// inputs. The codec is allocation-free on the encode path; the
    /// decode path allocates only the resulting value.
    #[test]
    fn json_round_trip_recovers_original(
        value in any::<i64>(),
        name in "[a-zA-Z0-9 _\\-]{0,64}",
        flags in proptest::collection::vec(any::<bool>(), 0..32),
    ) {
        let original = Msg { value, name, flags };
        let codec = JsonCodec::new();
        let mut buf = vec![0u8; 4096];
        let len = codec.encode(&original, &mut buf).expect("encode");
        prop_assert!(len <= buf.len());
        let decoded: Msg = codec.decode(&buf[..len]).expect("decode");
        prop_assert_eq!(decoded, original);
    }

    /// Property: the encoded form is also a stable JSON document —
    /// decoding it twice (via two distinct typed targets) is fine.
    /// Sanity-check that the buffer slice we hand to `decode` is the
    /// actual encoded JSON (and not, say, the buffer's tail).
    #[test]
    fn encoded_slice_parses_as_generic_json(
        value in any::<i64>(),
        name in "[a-zA-Z0-9]{0,32}",
    ) {
        let original = Msg { value, name, flags: vec![] };
        let codec = JsonCodec::new();
        let mut buf = vec![0u8; 1024];
        let len = codec.encode(&original, &mut buf).expect("encode");
        let json: serde_json::Value =
            serde_json::from_slice(&buf[..len]).expect("valid JSON document");
        prop_assert!(json.is_object(), "expected JSON object, got {json}");
    }
}
