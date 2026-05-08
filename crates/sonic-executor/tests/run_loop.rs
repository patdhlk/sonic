#![allow(missing_docs)]
use core::time::Duration;
use iceoryx2::prelude::*;
use sonic_executor::{ControlFlow, Executor, ExecutorError, TriggerDeclarer, item_with_triggers};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}.{}.{n}", std::process::id())
}

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
    let topic = unique("sonic.test.run.sub");
    let ch = exec.channel::<Tick>(&topic).unwrap();
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
            let _ = publisher.send_copy(Tick(i));
            std::thread::sleep(Duration::from_millis(20));
        }
    });

    exec.run().unwrap();
    let _ = stop;
    assert!(counter.load(Ordering::SeqCst) >= 3);
}

#[test]
fn threaded_pool_executes_items_correctly() {
    // Exercises the pool barrier + SendItemPtr discipline. With
    // worker_threads(2) the run loop dispatches each fired interval
    // trigger to a pool worker, then barriers before re-attaching.
    let mut exec = Executor::builder().worker_threads(2).build().unwrap();
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

    exec.run_n(5).unwrap();
    assert_eq!(
        counter.load(Ordering::SeqCst),
        5,
        "threaded pool should fire item exactly 5 times under run_n(5)"
    );
}

#[test]
fn item_task_id_override_takes_precedence() {
    use sonic_executor::{Context, ExecuteResult};

    struct NamedItem;
    impl sonic_executor::ExecutableItem for NamedItem {
        fn declare_triggers(&mut self, d: &mut TriggerDeclarer<'_>) -> Result<(), ExecutorError> {
            d.interval(Duration::from_millis(20));
            Ok(())
        }
        fn execute(&mut self, ctx: &mut Context<'_>) -> ExecuteResult {
            ctx.stop_executor();
            Ok(ControlFlow::Continue)
        }
        fn task_id(&self) -> Option<&str> {
            Some("custom-from-item")
        }
    }

    let mut exec = Executor::builder().worker_threads(0).build().unwrap();
    let id = exec.add_with_id("user-supplied-id", NamedItem).unwrap();
    assert_eq!(
        id.as_str(),
        "custom-from-item",
        "ExecutableItem::task_id() override should win over user-supplied id"
    );
    exec.run().unwrap();
}
