#![allow(missing_docs)]

use core::time::Duration;
use sonic_executor::{ControlFlow, ExecutionMonitor, Executor, TaskId, item_with_triggers};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Default)]
struct RecordingMonitor {
    pre: AtomicU32,
    post: AtomicU32,
    times: Mutex<Vec<(TaskId, Duration, bool)>>,
}

impl ExecutionMonitor for RecordingMonitor {
    fn pre_execute(&self, _: TaskId, _: Instant) {
        self.pre.fetch_add(1, Ordering::SeqCst);
    }
    fn post_execute(&self, t: TaskId, _: Instant, took: Duration, ok: bool) {
        self.post.fetch_add(1, Ordering::SeqCst);
        self.times.lock().unwrap().push((t, took, ok));
    }
}

#[test]
fn monitor_brackets_each_execute() {
    let mon = Arc::new(RecordingMonitor::default());
    let mut exec = Executor::builder()
        .worker_threads(0)
        .monitor(Arc::clone(&mon) as Arc<dyn ExecutionMonitor>)
        .build()
        .unwrap();

    exec.add(item_with_triggers(
        |d| {
            d.interval(Duration::from_millis(10));
            Ok(())
        },
        |_| Ok(ControlFlow::Continue),
    ))
    .unwrap();

    exec.run_n(3).unwrap();
    assert_eq!(mon.pre.load(Ordering::SeqCst), 3);
    assert_eq!(mon.post.load(Ordering::SeqCst), 3);
    assert!(mon.times.lock().unwrap().iter().all(|(_, _, ok)| *ok));
}
