#![allow(missing_docs)]

use core::time::Duration;
use sonic_executor::{ControlFlow, Executor, Observer, UserEvent};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct CountingObserver {
    up: AtomicU32,
    down: AtomicU32,
    err: AtomicU32,
    start: AtomicU32,
    stop: AtomicU32,
    user_events: Mutex<Vec<UserEvent>>,
}

impl Observer for CountingObserver {
    fn on_executor_up(&self) { self.up.fetch_add(1, Ordering::SeqCst); }
    fn on_executor_down(&self) { self.down.fetch_add(1, Ordering::SeqCst); }
    fn on_executor_error(&self, _: &sonic_executor::ExecutorError) {
        self.err.fetch_add(1, Ordering::SeqCst);
    }
    fn on_app_start(&self, _: sonic_executor::TaskId, _: u32, _: Option<u32>) {
        self.start.fetch_add(1, Ordering::SeqCst);
    }
    fn on_app_stop(&self, _: sonic_executor::TaskId) {
        self.stop.fetch_add(1, Ordering::SeqCst);
    }
    fn on_send_event(&self, _: sonic_executor::TaskId, ev: UserEvent) {
        self.user_events.lock().unwrap().push(ev);
    }
}

struct AppItem;

impl sonic_executor::ExecutableItem for AppItem {
    fn declare_triggers(
        &mut self,
        d: &mut sonic_executor::TriggerDeclarer<'_>,
    ) -> Result<(), sonic_executor::ExecutorError> {
        d.interval(Duration::from_millis(10));
        Ok(())
    }
    fn execute(
        &mut self,
        ctx: &mut sonic_executor::Context<'_>,
    ) -> sonic_executor::ExecuteResult {
        ctx.send_event(UserEvent { kind: 1, int_data: 42, string_data: None });
        Ok(ControlFlow::Continue)
    }
    fn app_id(&self) -> Option<u32> { Some(7) }
}

#[test]
fn observer_sees_lifecycle_and_user_events() {
    let obs = Arc::new(CountingObserver::default());
    let mut exec = Executor::builder()
        .worker_threads(0)
        .observer(Arc::clone(&obs) as Arc<dyn Observer>)
        .build()
        .unwrap();

    exec.add(AppItem).unwrap();
    exec.run_n(2).unwrap();

    assert_eq!(obs.up.load(Ordering::SeqCst), 1);
    assert_eq!(obs.down.load(Ordering::SeqCst), 1);
    assert!(obs.start.load(Ordering::SeqCst) >= 1);
    assert!(obs.stop.load(Ordering::SeqCst) >= 1);
    assert!(obs.user_events.lock().unwrap().iter().any(|e| e.int_data == 42));
}
