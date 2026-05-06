#![allow(missing_docs)]
use core::time::Duration;
use iceoryx2::prelude::*;
use sonic_executor::{item_with_triggers, ControlFlow, Executor};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

#[test]
fn interval_trigger_fires_run_n_times() {
    let mut exec = Executor::builder().worker_threads(0).build().unwrap();
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);

    exec.add(item_with_triggers(
        |d| {
            d.interval(Duration::from_millis(20));
            Ok(())
        },
        move |_| {
            c.fetch_add(1, Ordering::SeqCst);
            Ok(ControlFlow::Continue)
        },
    ))
    .unwrap();

    exec.run_n(3).unwrap();

    assert_eq!(counter.load(Ordering::SeqCst), 3);
}

#[test]
fn run_for_terminates_on_timeout() {
    let mut exec = Executor::builder().worker_threads(0).build().unwrap();
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);

    exec.add(item_with_triggers(
        |d| {
            d.interval(Duration::from_millis(50));
            Ok(())
        },
        move |_| {
            c.fetch_add(1, Ordering::SeqCst);
            Ok(ControlFlow::Continue)
        },
    ))
    .unwrap();

    exec.run_for(Duration::from_millis(120)).unwrap();
    assert!(counter.load(Ordering::SeqCst) >= 1);
}

#[test]
fn stoppable_terminates_run() {
    let mut exec = Executor::builder().worker_threads(0).build().unwrap();
    // NOTE: The `stop` handle obtained here via exec.stoppable() is NOT bound
    // to the run started by exec.run() below, because run_inner resets
    // self.stoppable = Stoppable::new(). The test works because the item calls
    // ctx.stop_executor(), which references the *current* Stoppable created
    // inside run_inner. Task 9 will re-architect the Stoppable to propagate to
    // existing clones.
    let stop = exec.stoppable();
    exec.add(item_with_triggers(
        |d| {
            d.interval(Duration::from_millis(20));
            Ok(())
        },
        move |ctx| {
            ctx.stop_executor();
            Ok(ControlFlow::Continue)
        },
    ))
    .unwrap();
    exec.run().unwrap();
    let _ = stop;
}

#[derive(Debug, Default, Clone, Copy, ZeroCopySend)]
#[repr(C)]
struct Tick(u64);

#[test]
fn subscriber_trigger_dispatches_task() {
    let mut exec = Executor::builder().worker_threads(0).build().unwrap();
    let ch = exec.channel::<Tick>("sonic.test.run.sub").unwrap();
    let publisher = ch.publisher().unwrap();
    let subscriber = ch.subscriber().unwrap();

    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let stop = exec.stoppable();

    exec.add(item_with_triggers(
        move |d| {
            d.subscriber(&subscriber);
            Ok(())
        },
        move |ctx| {
            c.fetch_add(1, Ordering::SeqCst);
            if c.load(Ordering::SeqCst) >= 3 {
                ctx.stop_executor();
            }
            Ok(ControlFlow::Continue)
        },
    ))
    .unwrap();

    std::thread::spawn(move || {
        for i in 0..5 {
            publisher.send_copy(Tick(i)).unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }
    });

    exec.run().unwrap();
    let _ = stop;
    assert!(counter.load(Ordering::SeqCst) >= 3);
}
