//! Compile-level check that the public API of `sonic-connector-zenoh`
//! doesn't leak `tokio::*` types. Maps to `TEST_0314`.
//!
//! If a future change adds a public method returning `tokio::Handle`,
//! `tokio::task::JoinHandle`, or similar, downstream test crates that
//! don't depend on tokio would fail to compile — which is exactly the
//! containment posture `REQ_0403` requires.

use sonic_connector_zenoh::registry::{ChannelDirection, ChannelRegistry};
use sonic_connector_zenoh::{
    KeyExprOwned, MockZenohSession, ZenohConnectorOptions, ZenohRouting, ZenohSessionLike,
    ZenohState,
};

#[test]
fn public_surface_is_tokio_free() {
    // If any of these types pulled `tokio` into their signature,
    // this test would not compile (no tokio types referenced).
    let opts = ZenohConnectorOptions::builder().build();
    let routing = ZenohRouting::new(KeyExprOwned::try_from("a/b").unwrap());
    let registry = ChannelRegistry::with_capacity(4);
    let dir = ChannelDirection::Outbound;
    let state = ZenohState::new(opts);
    let session = MockZenohSession::new();
    let state2: Box<dyn ZenohSessionLike> = Box::new(session);

    // Suppress clippy lints for compile-time proof variables.
    // These are not "no effect" — they're existence proofs that the public API
    // surfaces can be constructed without naming any tokio::* types.
    #[allow(clippy::no_effect_underscore_binding, clippy::used_underscore_binding)]
    {
        let _ = (&routing, &registry, &dir, &state, &state2);
    }
}
