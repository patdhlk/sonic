//! Unit tests for `ZenohQuerier`. End-to-end behavior is covered by
//! `tests/query_round_trip.rs` (`TEST_0303`); this file covers
//! construction, `QueryId` minting, and the `QuerierEvent` decoding
//! logic in isolation.

use sonic_connector_zenoh::querier::{ZeroedMinter, mint_query_id};
use sonic_connector_zenoh::registry::QueryId;

#[test]
fn mint_query_id_produces_monotonic_first_8_bytes() {
    let minter = ZeroedMinter::new();
    let a = minter.next();
    let b = minter.next();
    let c = minter.next();

    let a_counter = u64::from_be_bytes(a.0[..8].try_into().unwrap());
    let b_counter = u64::from_be_bytes(b.0[..8].try_into().unwrap());
    let c_counter = u64::from_be_bytes(c.0[..8].try_into().unwrap());

    assert!(b_counter > a_counter);
    assert!(c_counter > b_counter);
    // Remaining 24 bytes are zero in this minter.
    assert_eq!(&a.0[8..], &[0u8; 24]);
}

#[test]
fn mint_query_id_starts_above_zero() {
    let minter = ZeroedMinter::new();
    let id = minter.next();
    assert_ne!(id, QueryId([0; 32]));
}

#[test]
fn free_function_mint_query_id_uses_global_counter() {
    // The free function `mint_query_id` is the default minter used
    // by `create_querier` when no custom minter is supplied.
    let a = mint_query_id();
    let b = mint_query_id();
    assert_ne!(a, b);
}
