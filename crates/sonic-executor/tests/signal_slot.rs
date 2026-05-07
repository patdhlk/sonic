#![allow(missing_docs)]

use core::time::Duration;
use sonic_executor::{ControlFlow, Executor, item_with_triggers, signal_slot};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}.{}.{n}", std::process::id())
}

#[test]
fn unit_signal_to_slot_fires_chain() {
    let mut exec = Executor::builder().worker_threads(0).build().unwrap();
    let topic = unique("sonic.test.signal_slot.A");
    let (signal, slot) = signal_slot::pair::<u32>(&mut exec, &topic).unwrap();

    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);

    let head = item_with_triggers(
        |d| {
            d.interval(Duration::from_millis(10));
            Ok(())
        },
        |_| Ok(ControlFlow::Continue),
    );
    let signal = signal.before_send(|payload: &mut u32| {
        *payload = 7;
        true
    });
    let slot = slot.after_recv(move |p: &u32| {
        c.fetch_add(*p, Ordering::SeqCst);
        true
    });

    let chain: Vec<Box<dyn sonic_executor::ExecutableItem>> =
        vec![Box::new(head), Box::new(signal)];
    exec.add_chain(chain).unwrap();
    exec.add(slot).unwrap();

    exec.run_n(2).unwrap();
    assert!(counter.load(Ordering::SeqCst) >= 7);
}
