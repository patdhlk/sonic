//! Real-zenoh integration tests. Gated behind the `zenoh-integration`
//! cargo feature; not part of the default test run.
//!
//! - `TEST_0312` — two-peer real session query round-trip. Mirrors
//!   `TEST_0303` (which uses `MockZenohSession`) but over the real
//!   zenoh transport, with two peer-mode sessions on TCP loopback
//!   (random ports) in the same process.
//! - `TEST_0313` — client-mode session connecting to a router at
//!   `ZENOH_TEST_ROUTER`. Skipped when the env var is absent.

#![cfg(feature = "zenoh-integration")]

use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;

use sonic_connector_codec::JsonCodec;
use sonic_connector_core::ChannelDescriptor;
use sonic_connector_host::Connector;
use sonic_connector_zenoh::session::{SessionState, ZenohSessionLike};
use sonic_connector_zenoh::{
    KeyExprOwned, Locator, QuerierEvent, RealZenohSession, SessionMode, ZenohConnector,
    ZenohConnectorOptions, ZenohRouting, ZenohState,
};
use sonic_executor::Executor;

const N: usize = 512;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_0312_two_peer_real_session() {
    let port_a = pick_free_tcp_port();
    let port_b = pick_free_tcp_port();
    let endpoint_a = format!("tcp/127.0.0.1:{port_a}");
    let endpoint_b = format!("tcp/127.0.0.1:{port_b}");

    // Peer A — listens on endpoint_a, connects to endpoint_b.
    let opts_a = ZenohConnectorOptions::builder()
        .mode(SessionMode::Peer)
        .listen(Locator::new(&endpoint_a))
        .connect(Locator::new(&endpoint_b))
        .dispatcher_tick(Duration::from_millis(5))
        .query_timeout(Duration::from_secs(10))
        .tokio_worker_threads(2)
        .build();
    let session_a = Arc::new(RealZenohSession::open(&opts_a).await.unwrap());
    let state_a = Arc::new(ZenohState::new(opts_a));
    let mut conn_a = ZenohConnector::new(state_a, session_a, JsonCodec).unwrap();

    // Peer B — listens on endpoint_b, connects to endpoint_a.
    let opts_b = ZenohConnectorOptions::builder()
        .mode(SessionMode::Peer)
        .listen(Locator::new(&endpoint_b))
        .connect(Locator::new(&endpoint_a))
        .dispatcher_tick(Duration::from_millis(5))
        .query_timeout(Duration::from_secs(10))
        .tokio_worker_threads(2)
        .build();
    let session_b = Arc::new(RealZenohSession::open(&opts_b).await.unwrap());
    let state_b = Arc::new(ZenohState::new(opts_b));
    let mut conn_b = ZenohConnector::new(state_b, session_b, JsonCodec).unwrap();

    let routing = ZenohRouting::new(KeyExprOwned::try_from("test/0312/q").unwrap());
    let desc_a = ChannelDescriptor::<ZenohRouting, N>::new("test.0312".to_string(), routing.clone())
        .unwrap();
    let desc_b =
        ChannelDescriptor::<ZenohRouting, N>::new("test.0312".to_string(), routing).unwrap();

    // `create_queryable` / `create_querier` / `register_with` block on
    // the connector's internal gateway runtime (`Handle::block_on`).
    // Calling `block_on` from inside the surrounding `#[tokio::test]`
    // runtime would panic ("Cannot start a runtime from within a
    // runtime"), so we route the whole connector wiring through
    // `spawn_blocking`, which detaches to a non-runtime worker thread.
    let (mut qable, mut querier) = tokio::task::spawn_blocking(move || {
        let qable = conn_a.create_queryable::<u32, String, N>(&desc_a).unwrap();
        let querier = conn_b.create_querier::<u32, String, N>(&desc_b).unwrap();
        let mut exec_a = Executor::builder().worker_threads(0).build().unwrap();
        let mut exec_b = Executor::builder().worker_threads(0).build().unwrap();
        conn_a.register_with(&mut exec_a).unwrap();
        conn_b.register_with(&mut exec_b).unwrap();
        // Keep the executors and connectors alive for the duration of
        // the test by leaking them; the test process exits immediately
        // after assertions, so cleanup is unnecessary.
        Box::leak(Box::new((conn_a, conn_b, exec_a, exec_b)));
        (qable, querier)
    })
    .await
    .unwrap();

    // Peer discovery + zenoh link establishment over TCP loopback. The
    // explicit `connect` endpoints make discovery nominally immediate,
    // but the TCP handshake + zenoh's hello/init exchange adds latency.
    tokio::time::sleep(Duration::from_secs(2)).await;

    let q_id = querier.send(&42_u32).expect("send query");

    // Drain the queryable in a blocking task — `try_recv` is sync and
    // calling it in a hot loop on the async runtime would block a
    // worker thread.
    let received = tokio::task::spawn_blocking(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if let Ok(Some(pair)) = qable.try_recv() {
                qable
                    .reply(pair.0, &"hello from real zenoh".to_string())
                    .unwrap();
                qable.terminate(pair.0).unwrap();
                return Some(pair);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
    })
    .await
    .unwrap();
    assert!(received.is_some(), "queryable did not receive query within 5s");

    let observed = tokio::task::spawn_blocking(move || {
        let mut reply = None;
        let mut saw_eos = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !saw_eos && std::time::Instant::now() < deadline {
            if let Ok(Some(event)) = querier.try_recv() {
                match event {
                    QuerierEvent::Reply { id, value } => {
                        assert_eq!(id, q_id);
                        reply = Some(value);
                    }
                    QuerierEvent::EndOfStream { id } => {
                        assert_eq!(id, q_id);
                        saw_eos = true;
                    }
                    QuerierEvent::Timeout { .. } => panic!("unexpected timeout"),
                }
            } else {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        (reply, saw_eos)
    })
    .await
    .unwrap();

    assert_eq!(observed.0.as_deref(), Some("hello from real zenoh"));
    assert!(observed.1, "expected EndOfStream");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_0313_client_mode_router_smoke() {
    let Ok(router_endpoint) = std::env::var("ZENOH_TEST_ROUTER") else {
        eprintln!("Skipping TEST_0313: set ZENOH_TEST_ROUTER to enable");
        return;
    };

    let opts = ZenohConnectorOptions::builder()
        .mode(SessionMode::Client)
        .connect(Locator::new(&router_endpoint))
        .tokio_worker_threads(2)
        .build();
    let session = Arc::new(RealZenohSession::open(&opts).await.unwrap());

    let state = ZenohSessionLike::state(session.as_ref());
    assert!(
        matches!(state, SessionState::Alive),
        "session expected Alive, got {state:?}"
    );
}

fn pick_free_tcp_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}
