//! Tests for the Zenoh-side channel registry.
//!
//! Verifies the registry stores per-channel routing + iceoryx2
//! bindings and iterates them in insertion order without per-iter heap
//! allocation. Mirrors `sonic_connector_ethercat::registry` tests.

use std::sync::{Arc, Mutex};

use sonic_connector_core::ConnectorError;
use sonic_connector_zenoh::registry::{
    ChannelBinding, ChannelDirection, ChannelRegistry, CorrelatedPublish, InboundPublish,
    OutboundDrain, QuerierDrain, QueryId, ReplyDrain,
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
    let drain: Arc<dyn OutboundDrain> = Arc::new(CountingDrain {
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
        let drain: Arc<dyn OutboundDrain> = Arc::new(CountingDrain {
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
    let names: Vec<_> = registry
        .iter()
        .map(|e| e.descriptor_name.as_ref())
        .collect();
    assert_eq!(names, ["alpha", "beta", "gamma", "delta"]);
}

#[test]
fn duplicate_name_returns_error() {
    let mut registry = ChannelRegistry::with_capacity(4);
    let drain1: Arc<dyn OutboundDrain> = Arc::new(CountingDrain {
        calls: Mutex::new(0),
    });
    let drain2: Arc<dyn OutboundDrain> = Arc::new(CountingDrain {
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
    assert!(
        msg.contains("robot/arm"),
        "error names the duplicate: {msg}"
    );
}

#[test]
fn separate_outbound_and_inbound_with_same_name_is_allowed() {
    let mut registry = ChannelRegistry::with_capacity(4);
    registry
        .register(
            "robot/arm".to_string(),
            routing("robot/arm"),
            ChannelDirection::Outbound,
            ChannelBinding::Outbound(Arc::new(CountingDrain {
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

struct NullQuerierDrain;
impl QuerierDrain for NullQuerierDrain {
    fn drain_query(
        &self,
        _dest: &mut [u8],
    ) -> Result<Option<(QueryId, usize, u32)>, ConnectorError> {
        Ok(None)
    }
}

struct NullReplyDrain;
impl ReplyDrain for NullReplyDrain {
    fn drain_reply(&self, _dest: &mut [u8]) -> Result<Option<(QueryId, usize)>, ConnectorError> {
        Ok(None)
    }
}

struct RecordingCorrelatedPublish {
    seen: Mutex<Vec<(QueryId, Vec<u8>)>>,
}
impl CorrelatedPublish for RecordingCorrelatedPublish {
    fn publish_with_correlation(&self, id: QueryId, bytes: &[u8]) -> Result<(), ConnectorError> {
        self.seen.lock().unwrap().push((id, bytes.to_vec()));
        Ok(())
    }
}

#[test]
fn registry_records_querier_out_binding() {
    let mut registry = ChannelRegistry::with_capacity(2);
    registry
        .register(
            "robot/query".to_string(),
            routing("robot/query"),
            ChannelDirection::QuerierOut,
            ChannelBinding::QuerierOut(Arc::new(NullQuerierDrain)),
        )
        .expect("registered");
    let entry = registry.iter().next().unwrap();
    assert_eq!(entry.direction, ChannelDirection::QuerierOut);
    assert!(matches!(entry.binding, ChannelBinding::QuerierOut(_)));
}

#[test]
fn registry_records_querier_reply_in_binding() {
    let mut registry = ChannelRegistry::with_capacity(2);
    let publish = Box::new(RecordingCorrelatedPublish {
        seen: Mutex::new(Vec::new()),
    });
    registry
        .register(
            "robot/query".to_string(),
            routing("robot/query"),
            ChannelDirection::QuerierReplyIn,
            ChannelBinding::QuerierReplyIn(publish),
        )
        .expect("registered");
    let entry = registry.iter().next().unwrap();
    assert!(matches!(entry.binding, ChannelBinding::QuerierReplyIn(_)));
}

#[test]
fn registry_records_queryable_query_in_binding() {
    let mut registry = ChannelRegistry::with_capacity(2);
    let publish = Box::new(RecordingCorrelatedPublish {
        seen: Mutex::new(Vec::new()),
    });
    registry
        .register(
            "robot/query".to_string(),
            routing("robot/query"),
            ChannelDirection::QueryableQueryIn,
            ChannelBinding::QueryableQueryIn(publish),
        )
        .expect("registered");
    let entry = registry.iter().next().unwrap();
    assert!(matches!(entry.binding, ChannelBinding::QueryableQueryIn(_)));
}

#[test]
fn registry_records_queryable_reply_out_binding() {
    let mut registry = ChannelRegistry::with_capacity(2);
    registry
        .register(
            "robot/query".to_string(),
            routing("robot/query"),
            ChannelDirection::QueryableReplyOut,
            ChannelBinding::QueryableReplyOut(Arc::new(NullReplyDrain)),
        )
        .expect("registered");
    let entry = registry.iter().next().unwrap();
    assert!(matches!(
        entry.binding,
        ChannelBinding::QueryableReplyOut(_)
    ));
}

#[test]
fn query_id_round_trips_through_correlation_id_bytes() {
    let bytes = [
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
        0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E,
        0x1F, 0x20,
    ];
    let id = QueryId(bytes);
    assert_eq!(id.0, bytes);
    assert_eq!(id, QueryId(bytes));
    assert_ne!(id, QueryId([0; 32]));
}

#[test]
fn one_query_channel_registers_all_four_directions() {
    // A single query channel name registers four entries — querier
    // out/reply-in and queryable query-in/reply-out can all coexist
    // under one name.
    let mut registry = ChannelRegistry::with_capacity(4);
    let nm = "robot/query".to_string();
    let r = routing("robot/query");

    registry
        .register(
            nm.clone(),
            r.clone(),
            ChannelDirection::QuerierOut,
            ChannelBinding::QuerierOut(Arc::new(NullQuerierDrain)),
        )
        .unwrap();
    registry
        .register(
            nm.clone(),
            r.clone(),
            ChannelDirection::QuerierReplyIn,
            ChannelBinding::QuerierReplyIn(Box::new(RecordingCorrelatedPublish {
                seen: Mutex::new(Vec::new()),
            })),
        )
        .unwrap();
    registry
        .register(
            nm.clone(),
            r.clone(),
            ChannelDirection::QueryableQueryIn,
            ChannelBinding::QueryableQueryIn(Box::new(RecordingCorrelatedPublish {
                seen: Mutex::new(Vec::new()),
            })),
        )
        .unwrap();
    registry
        .register(
            nm,
            r,
            ChannelDirection::QueryableReplyOut,
            ChannelBinding::QueryableReplyOut(Arc::new(NullReplyDrain)),
        )
        .unwrap();

    assert_eq!(registry.len(), 4);
}
