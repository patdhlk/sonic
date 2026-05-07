#![allow(missing_docs)]
use iceoryx2::prelude::*;
use sonic_executor::Channel;
use std::sync::Arc;

static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{prefix}.{}.{n}", std::process::id())
}

#[derive(Debug, Default, Clone, Copy, ZeroCopySend)]
#[repr(C)]
struct Msg(u64);

#[test]
fn publisher_send_notifies_subscriber_listener() {
    let node = NodeBuilder::new().create::<ipc::Service>().unwrap();

    let channel: Arc<Channel<Msg>> =
        Channel::open_or_create(&node, &unique("sonic.test.chan")).unwrap();

    let publisher = channel.publisher().unwrap();
    let subscriber = channel.subscriber().unwrap();

    publisher.send_copy(Msg(42)).unwrap();

    // The subscriber's listener fires because Publisher::send notified.
    let listener = subscriber.listener_handle();
    let mut woke = 0_u32;
    while let Ok(Some(_)) = listener.try_wait_one() {
        woke += 1;
    }
    assert!(woke >= 1, "subscriber listener did not fire");

    let sample = subscriber.take().unwrap().expect("payload");
    assert_eq!(sample.payload().0, 42);
}

#[test]
fn opening_same_channel_twice_does_not_panic() {
    let node = NodeBuilder::new().create::<ipc::Service>().unwrap();
    let chan_name = unique("sonic.test.chan2");
    let _a: Arc<Channel<Msg>> = Channel::open_or_create(&node, &chan_name).unwrap();
    let _b: Arc<Channel<Msg>> = Channel::open_or_create(&node, &chan_name).unwrap();
    // No assertion — the call must not panic and must not deadlock.
}
