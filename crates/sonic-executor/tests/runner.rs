#![allow(missing_docs)]

use core::time::Duration;
use sonic_executor::{ControlFlow, Executor, Runner, RunnerFlags, item_with_triggers};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

#[test]
fn runner_runs_until_stop() {
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

    let mut runner = Runner::new(exec, RunnerFlags::empty()).unwrap();
    std::thread::sleep(Duration::from_millis(120));
    runner.stop().unwrap();
    assert!(counter.load(Ordering::SeqCst) >= 1);
}

#[test]
fn runner_deferred_does_not_run_until_started() {
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

    let mut runner = Runner::new(exec, RunnerFlags::DEFERRED).unwrap();
    std::thread::sleep(Duration::from_millis(60));
    assert_eq!(
        counter.load(Ordering::SeqCst),
        0,
        "deferred runner ran prematurely"
    );

    runner.start().unwrap();
    std::thread::sleep(Duration::from_millis(80));
    runner.stop().unwrap();
    assert!(counter.load(Ordering::SeqCst) >= 1);
}

#[test]
fn runner_stop_rethrows_item_error() {
    let mut exec = Executor::builder().worker_threads(0).build().unwrap();
    exec.add(item_with_triggers(
        |d| {
            d.interval(Duration::from_millis(10));
            Ok(())
        },
        |_| Err(Box::new(std::io::Error::other("boom"))),
    ))
    .unwrap();

    let mut runner = Runner::new(exec, RunnerFlags::empty()).unwrap();
    let res = {
        std::thread::sleep(Duration::from_millis(40));
        runner.stop()
    };
    let err = res.expect_err("runner should re-throw item error");
    assert!(format!("{err}").contains("boom"));
}

#[test]
fn drop_without_stop_does_not_panic() {
    let mut exec = Executor::builder().worker_threads(0).build().unwrap();
    exec.add(item_with_triggers(
        |d| {
            d.interval(Duration::from_millis(10));
            Ok(())
        },
        |_| Err(Box::new(std::io::Error::other("boom"))),
    ))
    .unwrap();

    let runner = Runner::new(exec, RunnerFlags::empty()).unwrap();
    std::thread::sleep(Duration::from_millis(40));
    drop(runner); // must not panic
}
