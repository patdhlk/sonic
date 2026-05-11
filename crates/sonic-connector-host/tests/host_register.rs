//! Exercises the host's register → run loop with a minimal in-tree
//! connector. The host must (a) accept any `Connector` impl,
//! (b) call `register_with`, (c) drive the registered
//! `ExecutableItem` on its executor — all of `REQ_0270`–`REQ_0272`.

#![allow(clippy::doc_markdown)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, unbounded};
use sonic_connector_codec::JsonCodec;
use sonic_connector_core::{
    ChannelDescriptor, ConnectorError, ConnectorHealth, ConnectorHealthKind, HealthEvent, Routing,
};
use sonic_connector_host::{Connector, ConnectorHost, HealthSubscription};
use sonic_connector_transport_iox::{ChannelReader, ChannelWriter};
use sonic_executor::{ControlFlow, Executor, item_with_triggers};

#[derive(Clone, Debug)]
struct EchoRouting;

impl Routing for EchoRouting {}

/// Tracks how many times the connector's executable item ran, and
/// holds the sender end of a health channel for `subscribe_health`.
struct EchoState {
    ticks: AtomicU64,
    health_tx: Sender<HealthEvent>,
    health_rx: Receiver<HealthEvent>,
}

struct EchoConnector {
    state: Arc<EchoState>,
}

impl EchoConnector {
    fn new() -> Self {
        let (health_tx, health_rx) = unbounded();
        Self {
            state: Arc::new(EchoState {
                ticks: AtomicU64::new(0),
                health_tx,
                health_rx,
            }),
        }
    }

    fn ticks(&self) -> u64 {
        self.state.ticks.load(Ordering::SeqCst)
    }
}

impl Connector for EchoConnector {
    type Routing = EchoRouting;
    type Codec = JsonCodec;

    fn name(&self) -> &'static str {
        "echo"
    }

    fn health(&self) -> ConnectorHealth {
        ConnectorHealth::Up
    }

    fn subscribe_health(&self) -> HealthSubscription {
        HealthSubscription::new(self.state.health_rx.clone())
    }

    fn register_with(&mut self, executor: &mut Executor) -> Result<(), ConnectorError> {
        let state = Arc::clone(&self.state);
        let item = item_with_triggers(
            |d| {
                d.interval(Duration::from_millis(1));
                Ok(())
            },
            move |_ctx| {
                state.ticks.fetch_add(1, Ordering::SeqCst);
                Ok(ControlFlow::Continue)
            },
        );
        executor.add(item).map_err(ConnectorError::stack)?;
        Ok(())
    }

    fn create_writer<T, const N: usize>(
        &self,
        _descriptor: &ChannelDescriptor<Self::Routing, N>,
    ) -> Result<ChannelWriter<T, Self::Codec, N>, ConnectorError>
    where
        T: serde::Serialize,
    {
        Err(ConnectorError::InvalidDescriptor(
            "EchoConnector::create_writer not implemented in this test fixture".into(),
        ))
    }

    fn create_reader<T, const N: usize>(
        &self,
        _descriptor: &ChannelDescriptor<Self::Routing, N>,
    ) -> Result<ChannelReader<T, Self::Codec, N>, ConnectorError>
    where
        T: serde::de::DeserializeOwned,
    {
        Err(ConnectorError::InvalidDescriptor(
            "EchoConnector::create_reader not implemented in this test fixture".into(),
        ))
    }
}

/// Build a representative `HealthEvent` for round-trip checks.
fn make_health_event() -> HealthEvent {
    HealthEvent {
        from: ConnectorHealth::Connecting {
            since: std::time::Instant::now(),
        },
        to: ConnectorHealth::Up,
        at: std::time::Instant::now(),
    }
}

#[test]
fn host_drives_registered_connector_executable_item() {
    let mut host = ConnectorHost::builder().worker_threads(0).build().unwrap();
    let connector = host.register(EchoConnector::new()).expect("register");

    // Run a handful of barrier-cycles — each is one WaitSet wakeup,
    // which fires the interval trigger and runs the closure once.
    host.run_n(3).expect("run_n");

    let ticks = connector.ticks();
    assert!(
        ticks >= 1,
        "expected at least one tick after run_n(3), got {ticks}"
    );
}

#[test]
fn host_records_registered_connector_name() {
    let mut host = ConnectorHost::builder().worker_threads(0).build().unwrap();
    let _ = host.register(EchoConnector::new()).unwrap();
    assert_eq!(host.connector_names(), &["echo".to_string()]);
}

#[test]
fn health_subscription_observes_published_events() {
    let host = ConnectorHost::builder().worker_threads(0).build().unwrap();
    let _ = host;
    let connector = EchoConnector::new();
    let sub = connector.subscribe_health();

    let ev = make_health_event();
    connector
        .state
        .health_tx
        .send(ev)
        .expect("send on internal health channel");

    let observed = sub
        .try_next()
        .expect("no disconnection")
        .expect("event was available");
    assert_eq!(observed.from.kind(), ConnectorHealthKind::Connecting);
    assert_eq!(observed.to.kind(), ConnectorHealthKind::Up);
}
