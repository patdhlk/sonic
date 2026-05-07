#![allow(missing_docs)]

use core::time::Duration;
use sonic_executor::{item, item_with_triggers, ControlFlow, Executor};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

#[test]
fn chain_runs_items_in_order() {
    let mut exec = Executor::builder().worker_threads(2).build().unwrap();
    let log = Arc::new(std::sync::Mutex::new(Vec::<u32>::new()));

    let l1 = Arc::clone(&log);
    let head = item_with_triggers(
        |d| {
            d.interval(Duration::from_millis(10));
            Ok(())
        },
        move |_| {
            l1.lock().unwrap().push(1);
            Ok(ControlFlow::Continue)
        },
    );

    let l2 = Arc::clone(&log);
    let mid = item(move |_| {
        l2.lock().unwrap().push(2);
        Ok(ControlFlow::Continue)
    });
    let l3 = Arc::clone(&log);
    let tail = item(move |_| {
        l3.lock().unwrap().push(3);
        Ok(ControlFlow::Continue)
    });

    let chain: Vec<Box<dyn sonic_executor::ExecutableItem>> =
        vec![Box::new(head), Box::new(mid), Box::new(tail)];
    exec.add_chain(chain).unwrap();

    exec.run_n(1).unwrap();
    let l = log.lock().unwrap().clone();
    assert_eq!(l, vec![1, 2, 3]);
}

#[test]
fn stop_chain_aborts_remaining_items() {
    let mut exec = Executor::builder().worker_threads(0).build().unwrap();
    let counter = Arc::new(AtomicU32::new(0));

    let c1 = Arc::clone(&counter);
    let head = item_with_triggers(
        |d| {
            d.interval(Duration::from_millis(10));
            Ok(())
        },
        move |_| {
            c1.fetch_add(1, Ordering::SeqCst);
            Ok(ControlFlow::StopChain)
        },
    );
    let c2 = Arc::clone(&counter);
    let tail = item(move |_| {
        c2.fetch_add(1, Ordering::SeqCst);
        Ok(ControlFlow::Continue)
    });

    let chain: Vec<Box<dyn sonic_executor::ExecutableItem>> = vec![Box::new(head), Box::new(tail)];
    exec.add_chain(chain).unwrap();

    exec.run_n(1).unwrap();
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "tail must not run after StopChain"
    );
}

#[test]
fn err_in_middle_propagates_and_stops() {
    let mut exec = Executor::builder().worker_threads(0).build().unwrap();
    let head = item_with_triggers(
        |d| {
            d.interval(Duration::from_millis(10));
            Ok(())
        },
        |_| Ok(ControlFlow::Continue),
    );
    let mid = item(|_| Err(Box::new(std::io::Error::other("mid-err"))));
    let tail_seen = Arc::new(AtomicU32::new(0));
    let t = Arc::clone(&tail_seen);
    let tail = item(move |_| {
        t.fetch_add(1, Ordering::SeqCst);
        Ok(ControlFlow::Continue)
    });

    let chain: Vec<Box<dyn sonic_executor::ExecutableItem>> =
        vec![Box::new(head), Box::new(mid), Box::new(tail)];
    exec.add_chain(chain).unwrap();

    let err = exec.run_n(1).expect_err("expected chain error");
    assert!(format!("{err}").contains("mid-err"));
    assert_eq!(tail_seen.load(Ordering::SeqCst), 0);
}
