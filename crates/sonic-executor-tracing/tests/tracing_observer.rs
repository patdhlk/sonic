#![allow(missing_docs)]

use core::time::Duration;
use sonic_executor::{item_with_triggers, ControlFlow, Executor, Observer, UserEvent};
use sonic_executor_tracing::TracingObserver;
use std::sync::Arc;

#[test]
fn tracing_observer_runs_without_panic() {
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .finish();

    tracing::subscriber::with_default(subscriber, || {
        let obs: Arc<dyn Observer> = Arc::new(TracingObserver);
        let mut exec = Executor::builder()
            .worker_threads(0)
            .install_ctrlc(false)
            .observer(obs)
            .build()
            .unwrap();
        exec.add(item_with_triggers(
            |d| { d.interval(Duration::from_millis(10)); Ok(()) },
            |ctx| {
                ctx.send_event(UserEvent {
                    kind: 1,
                    int_data: 7,
                    string_data: Some("hi".into()),
                });
                Ok(ControlFlow::Continue)
            },
        )).unwrap();
        exec.run_n(1).unwrap();
    });
}
