//! Unit tests for `ZenohQueryable`. The handle's behavior is hard to
//! exercise in isolation because every method depends on the iox
//! services being wired through the connector. End-to-end behavior
//! lives in `tests/query_round_trip.rs` (`TEST_0303`); this file
//! covers compile-time type assertions.

use sonic_connector_codec::JsonCodec;
use sonic_connector_zenoh::{ZenohQuerier, ZenohQueryable};

/// Compile-check: queryable type takes the same generics shape as
/// querier (`<Q, R, C, N>`), even though Q and R are swapped in their
/// trait bounds (Q is now `DeserializeOwned`, R is `Serialize`).
#[test]
fn queryable_generics_compose() {
    fn assert_queryable<Q, R, C, const N: usize>()
    where
        Q: serde::de::DeserializeOwned,
        R: serde::Serialize,
        C: sonic_connector_core::PayloadCodec,
    {
        let _: Option<ZenohQueryable<Q, R, C, N>> = None;
    }
    assert_queryable::<u32, u32, JsonCodec, 256>();
    assert_queryable::<String, String, JsonCodec, 1024>();
}

#[test]
fn querier_and_queryable_share_codec_and_capacity_generics() {
    fn assert_compat<Q, R, C, const N: usize>()
    where
        Q: serde::Serialize + serde::de::DeserializeOwned,
        R: serde::Serialize + serde::de::DeserializeOwned,
        C: sonic_connector_core::PayloadCodec,
    {
        let _: Option<ZenohQuerier<Q, R, C, N>> = None;
        let _: Option<ZenohQueryable<Q, R, C, N>> = None;
    }
    assert_compat::<u32, String, JsonCodec, 512>();
}
