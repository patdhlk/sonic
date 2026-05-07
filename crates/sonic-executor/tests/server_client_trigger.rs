#![allow(missing_docs)]

use core::time::Duration;
use iceoryx2::prelude::*;
use sonic_executor::{item_with_triggers, ControlFlow, Executor};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

static SEQ: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}.{}.{n}", std::process::id())
}

#[derive(Debug, Default, Clone, Copy, ZeroCopySend)]
#[repr(C)]
struct Req(u64);

#[derive(Debug, Default, Clone, Copy, ZeroCopySend)]
#[repr(C)]
struct Resp(u64);

#[test]
fn server_trigger_fires_when_request_arrives() {
    let mut exec = Executor::builder().worker_threads(0).build().unwrap();
    let svc = exec
        .service::<Req, Resp>(&unique("sonic.test.svc.trig"))
        .unwrap();
    let server = svc.server().unwrap();
    let client = svc.client().unwrap();

    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let stop = exec.stoppable();

    exec.add(item_with_triggers(
        move |d| {
            d.server(&server);
            Ok(())
        },
        move |ctx| {
            c.fetch_add(1, Ordering::SeqCst);
            if c.load(Ordering::SeqCst) >= 2 {
                ctx.stop_executor();
            }
            Ok(ControlFlow::Continue)
        },
    ))
    .unwrap();

    std::thread::spawn(move || {
        for i in 0..3 {
            let _ = client.send_copy(Req(i));
            std::thread::sleep(Duration::from_millis(20));
        }
    });

    exec.run().unwrap();
    let _ = stop;
    assert!(counter.load(Ordering::SeqCst) >= 2);
}
