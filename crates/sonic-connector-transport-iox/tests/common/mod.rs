//! Shared scaffolding for the transport-iox integration tests.
//!
//! Each test crate-test owns its iceoryx2 [`Node`] with a unique name
//! (derived from the test function) so cross-test service collisions
//! cannot occur. Names are short and ASCII-only — iceoryx2's service
//! naming has length and character-set caps that ad-hoc UUIDs would
//! risk overflowing.

#![allow(dead_code)] // shared helpers — not every test uses every item

use std::sync::atomic::{AtomicU64, Ordering};

use iceoryx2::node::Node;
use iceoryx2::prelude::{NodeBuilder, ipc};
use serde::{Deserialize, Serialize};
use sonic_connector_core::{ChannelDescriptor, ConnectorError, PayloadCodec, Routing};

/// Construct a fresh iceoryx2 node. Each test should call this once.
pub fn make_node() -> Node<ipc::Service> {
    NodeBuilder::new()
        .create::<ipc::Service>()
        .expect("create iceoryx2 node")
}

/// Process-wide counter so concurrently-running test BINARIES (cargo
/// runs `tests/*.rs` as separate binaries by default) still pick
/// disjoint channel names when invoked with `--test-threads=1`.
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Mint a channel name unique to this process invocation. Combines a
/// short caller-supplied tag with a monotonic counter.
pub fn unique_channel_name(tag: &str) -> String {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("sntx.{tag}.{n}")
}

/// Minimal `Routing` impl for tests — the framework never inspects it
/// at the transport layer.
#[derive(Clone, Debug)]
pub struct TestRouting;

impl Routing for TestRouting {}

/// Test payload type — serde-derived so we don't depend on a real codec
/// implementation yet.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Msg {
    pub value: u32,
    pub note: String,
}

/// `serde_json`-backed codec used by tests. Production code uses
/// `JsonCodec` from `sonic-connector-codec` (lands in C3); this stub
/// keeps the transport tests independent of that crate.
#[derive(Clone, Copy, Debug, Default)]
pub struct TestJsonCodec;

impl PayloadCodec for TestJsonCodec {
    fn format_name(&self) -> &'static str {
        "test-json"
    }

    fn encode<T>(&self, value: &T, buf: &mut [u8]) -> Result<usize, ConnectorError>
    where
        T: serde::Serialize,
    {
        let bytes = serde_json::to_vec(value).map_err(|e| ConnectorError::codec("test-json", e))?;
        if bytes.len() > buf.len() {
            return Err(ConnectorError::PayloadOverflow {
                actual: bytes.len(),
                max: buf.len(),
            });
        }
        buf[..bytes.len()].copy_from_slice(&bytes);
        Ok(bytes.len())
    }

    fn decode<T>(&self, buf: &[u8]) -> Result<T, ConnectorError>
    where
        T: serde::de::DeserializeOwned,
    {
        serde_json::from_slice(buf).map_err(|e| ConnectorError::codec("test-json", e))
    }
}

/// Build a fresh `ChannelDescriptor` with a unique name for `tag`.
pub fn descriptor<const N: usize>(tag: &str) -> ChannelDescriptor<TestRouting, N> {
    ChannelDescriptor::new(unique_channel_name(tag), TestRouting).expect("non-empty name")
}
