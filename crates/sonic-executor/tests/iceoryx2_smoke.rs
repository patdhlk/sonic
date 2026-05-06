//! Verifies the iceoryx2 API surface this crate depends on.
//! If this test fails to compile, the iceoryx2 version pinned in the
//! workspace manifest has shifted shapes and the rest of the plan needs
//! to be adapted.

use core::time::Duration;
use iceoryx2::prelude::*;

#[derive(Debug, Default, Clone, Copy, ZeroCopySend)]
#[repr(C)]
struct Tick(u64);

#[test]
fn pubsub_event_waitset_round_trip() {
    let node = NodeBuilder::new()
        .create::<ipc::Service>()
        .expect("create node");

    // Publish-subscribe service.
    let pubsub = node
        .service_builder(&"sonic.smoke.tick".try_into().unwrap())
        .publish_subscribe::<Tick>()
        .open_or_create()
        .expect("create pubsub service");

    let publisher  = pubsub.publisher_builder().create().expect("publisher");
    let subscriber = pubsub.subscriber_builder().create().expect("subscriber");

    // Paired event service used to wake the WaitSet on send.
    let event = node
        .service_builder(&"sonic.smoke.tick.__sonic_event".try_into().unwrap())
        .event()
        .open_or_create()
        .expect("create event service");

    let notifier = event.notifier_builder().create().expect("notifier");
    let listener = event.listener_builder().create().expect("listener");

    // WaitSet attaches the listener.
    let waitset = WaitSetBuilder::new()
        .create::<ipc::Service>()
        .expect("waitset");
    let _guard = waitset.attach_notification(&listener).expect("attach listener");

    // Publisher sends, notifier wakes the waitset.
    publisher.send_copy(Tick(7)).expect("send");
    notifier.notify().expect("notify");

    // Drive the waitset for a bounded time.
    let mut got_event = false;
    let _interval = waitset
        .attach_interval(Duration::from_millis(50))
        .expect("attach interval");

    waitset
        .wait_and_process(|_| {
            // First wakeup is from the listener; we read and stop.
            while let Ok(Some(_)) = listener.try_wait_one() {
                got_event = true;
            }
            CallbackProgression::Stop
        })
        .expect("wait_and_process");

    assert!(got_event, "waitset did not wake on listener notify");

    // Subscriber sees the published payload.
    let sample = subscriber
        .receive()
        .expect("receive")
        .expect("sample present");
    assert_eq!(sample.payload().0, 7);
}
