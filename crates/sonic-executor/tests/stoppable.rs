#![allow(missing_docs)]

use core::time::Duration;
use sonic_executor::{item_with_triggers, ControlFlow, Executor};
use std::time::Instant;

/// Verify that a `Stoppable` clone obtained *before* `run()` is waker-aware
/// because the stop event is now wired at `build()` time (Option A). The
/// executor's `WaitSet` is attached to the stop listener, so calling `stop()`
/// from another thread wakes it even when its only trigger has a 60-second
/// interval (i.e. it would otherwise block indefinitely).
#[test]
fn stop_from_other_thread_wakes_idle_executor() {
    let mut exec = Executor::builder().worker_threads(0).build().unwrap();

    // Item with a *very* slow interval; without a wakeup the loop would block.
    exec.add(item_with_triggers(
        |d| {
            d.interval(Duration::from_secs(60));
            Ok(())
        },
        |_| Ok(ControlFlow::Continue),
    ))
    .unwrap();

    // Clone the Stoppable BEFORE run() — this is the key assertion: because
    // the stop event is wired at build() time, this clone already carries the
    // waker and will wake the WaitSet.
    let stop = exec.stoppable();

    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        stop.stop();
    });

    let start = Instant::now();
    exec.run().unwrap();
    t.join().unwrap();

    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "stoppable did not wake the loop promptly (elapsed = {elapsed:?})"
    );
}
