#![allow(missing_docs)]

use iceoryx2::prelude::*;
use sonic_executor::{Executor, Service};
use std::sync::Arc;

static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{prefix}.{}.{n}", std::process::id())
}

#[derive(Debug, Default, Clone, Copy, ZeroCopySend)]
#[repr(C)]
struct Req(f64);

#[derive(Debug, Default, Clone, Copy, ZeroCopySend)]
#[repr(C)]
struct Resp(f64);

#[test]
fn server_receives_request_via_listener() {
    let mut exec = Executor::builder().worker_threads(0).build().unwrap();
    let svc: Arc<Service<Req, Resp>> = exec.service(&unique("sonic.test.svc")).unwrap();

    let server = svc.server().unwrap();
    let client = svc.client().unwrap();

    let _pending = client.send_copy(Req(16.0)).unwrap();

    let listener = server.listener_handle();
    let mut woke = 0;
    while let Ok(Some(_)) = listener.try_wait_one() {
        woke += 1;
    }
    assert!(woke >= 1);

    let (req, active) = server.take_request().unwrap().expect("request");
    #[allow(clippy::float_cmp)]
    {
        assert_eq!(req.0, 16.0);
    }
    active.respond_copy(Resp(4.0)).unwrap();
}
