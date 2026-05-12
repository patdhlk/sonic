//! Tests for the Zenoh-side channel registry.
//!
//! Verifies the registry stores per-channel routing + iceoryx2
//! bindings and iterates them in insertion order without per-iter heap
//! allocation. Mirrors `sonic_connector_ethercat::registry` tests.

use std::sync::Mutex;

use sonic_connector_core::ConnectorError;
use sonic_connector_zenoh::registry::{
    ChannelBinding, ChannelDirection, ChannelRegistry, InboundPublish, OutboundDrain,
};
use sonic_connector_zenoh::{KeyExprOwned, ZenohRouting};

fn routing(key: &str) -> ZenohRouting {
    ZenohRouting::new(KeyExprOwned::try_from(key).unwrap())
}

struct CountingDrain {
    calls: Mutex<usize>,
}
impl OutboundDrain for CountingDrain {
    fn drain_into(&self, _dest: &mut [u8]) -> Result<Option<usize>, ConnectorError> {
        *self.calls.lock().unwrap() += 1;
        Ok(None)
    }
}

struct RecordingPublish {
    seen: Mutex<Vec<Vec<u8>>>,
}
impl InboundPublish for RecordingPublish {
    fn publish_bytes(&self, bytes: &[u8]) -> Result<(), ConnectorError> {
        self.seen.lock().unwrap().push(bytes.to_vec());
        Ok(())
    }
}

#[test]
fn empty_registry_iter_is_empty() {
    let registry = ChannelRegistry::with_capacity(8);
    assert_eq!(registry.iter().count(), 0);
    assert_eq!(registry.len(), 0);
}

#[test]
fn registry_records_outbound_channel() {
    let mut registry = ChannelRegistry::with_capacity(4);
    let drain = Box::new(CountingDrain {
        calls: Mutex::new(0),
    });

    registry
        .register(
            "robot/arm/joint1".to_string(),
            routing("robot/arm/joint1"),
            ChannelDirection::Outbound,
            ChannelBinding::Outbound(drain),
        )
        .expect("registered");

    assert_eq!(registry.len(), 1);
    let entry = registry.iter().next().expect("one entry");
    assert_eq!(entry.descriptor_name, "robot/arm/joint1");
    assert_eq!(entry.direction, ChannelDirection::Outbound);
    assert!(matches!(entry.binding, ChannelBinding::Outbound(_)));
}

#[test]
fn registry_records_inbound_channel() {
    let mut registry = ChannelRegistry::with_capacity(4);
    let publish = Box::new(RecordingPublish {
        seen: Mutex::new(Vec::new()),
    });

    registry
        .register(
            "robot/sensor/temp".to_string(),
            routing("robot/sensor/temp"),
            ChannelDirection::Inbound,
            ChannelBinding::Inbound(publish),
        )
        .expect("registered");

    let entry = registry.iter().next().expect("one entry");
    assert_eq!(entry.direction, ChannelDirection::Inbound);
    assert!(matches!(entry.binding, ChannelBinding::Inbound(_)));
}

#[test]
fn registry_iter_preserves_insertion_order() {
    let mut registry = ChannelRegistry::with_capacity(4);
    for n in ["alpha", "beta", "gamma", "delta"] {
        let drain = Box::new(CountingDrain {
            calls: Mutex::new(0),
        });
        registry
            .register(
                n.to_string(),
                routing(&format!("robot/{n}")),
                ChannelDirection::Outbound,
                ChannelBinding::Outbound(drain),
            )
            .unwrap();
    }
    let names: Vec<_> = registry.iter().map(|e| e.descriptor_name.as_ref()).collect();
    assert_eq!(names, ["alpha", "beta", "gamma", "delta"]);
}

#[test]
fn duplicate_name_returns_error() {
    let mut registry = ChannelRegistry::with_capacity(4);
    let drain1 = Box::new(CountingDrain {
        calls: Mutex::new(0),
    });
    let drain2 = Box::new(CountingDrain {
        calls: Mutex::new(0),
    });

    registry
        .register(
            "robot/arm".to_string(),
            routing("robot/arm"),
            ChannelDirection::Outbound,
            ChannelBinding::Outbound(drain1),
        )
        .unwrap();
    let err = registry
        .register(
            "robot/arm".to_string(),
            routing("robot/arm"),
            ChannelDirection::Outbound,
            ChannelBinding::Outbound(drain2),
        )
        .expect_err("dup rejected");
    let msg = err.to_string();
    assert!(msg.contains("robot/arm"), "error names the duplicate: {msg}");
}

#[test]
fn separate_outbound_and_inbound_with_same_name_is_allowed() {
    let mut registry = ChannelRegistry::with_capacity(4);
    registry
        .register(
            "robot/arm".to_string(),
            routing("robot/arm"),
            ChannelDirection::Outbound,
            ChannelBinding::Outbound(Box::new(CountingDrain {
                calls: Mutex::new(0),
            })),
        )
        .unwrap();
    registry
        .register(
            "robot/arm".to_string(),
            routing("robot/arm"),
            ChannelDirection::Inbound,
            ChannelBinding::Inbound(Box::new(RecordingPublish {
                seen: Mutex::new(Vec::new()),
            })),
        )
        .expect("inbound + outbound on same name OK");
    assert_eq!(registry.len(), 2);
}
