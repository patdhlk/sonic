#![allow(missing_docs)]
#![cfg(feature = "thread_attrs")]

use core::time::Duration;
use sonic_executor::{ControlFlow, Executor, ThreadAttributes, item_with_triggers};

#[test]
fn worker_attrs_compiles_and_runs() {
    let attrs = ThreadAttributes::new()
        .name_prefix("sonic-test")
        .affinity_mask(vec![0]); // pin to CPU 0

    let mut exec = Executor::builder()
        .worker_threads(2)
        .worker_attrs(attrs)
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

    exec.run_n(1).unwrap();
}
