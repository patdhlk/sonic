#![allow(missing_docs)]

use core::time::Duration;
use sonic_executor::{item, item_with_triggers, ControlFlow, Executor};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

#[test]
fn diamond_runs_all_vertices_once() {
    let mut exec = Executor::builder().worker_threads(2).build().unwrap();
    let counts = [0_u32; 4].map(|_| Arc::new(AtomicU32::new(0)));

    let mut g = exec.add_graph();
    let c0 = Arc::clone(&counts[0]);
    let r = g.vertex(item_with_triggers(
        |d| {
            d.interval(Duration::from_millis(10));
            Ok(())
        },
        move |_| {
            c0.fetch_add(1, Ordering::SeqCst);
            Ok(ControlFlow::Continue)
        },
    ));
    let c1 = Arc::clone(&counts[1]);
    let l = g.vertex(item(move |_| {
        c1.fetch_add(1, Ordering::SeqCst);
        Ok(ControlFlow::Continue)
    }));
    let c2 = Arc::clone(&counts[2]);
    let rt = g.vertex(item(move |_| {
        c2.fetch_add(1, Ordering::SeqCst);
        Ok(ControlFlow::Continue)
    }));
    let c3 = Arc::clone(&counts[3]);
    let m = g.vertex(item(move |_| {
        c3.fetch_add(1, Ordering::SeqCst);
        Ok(ControlFlow::Continue)
    }));
    g.edge(r, l).edge(r, rt).edge(l, m).edge(rt, m).root(r);
    g.build().unwrap();

    exec.run_n(1).unwrap();
    for c in &counts {
        assert_eq!(c.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn root_stop_chain_skips_dependents() {
    let mut exec = Executor::builder().worker_threads(2).build().unwrap();
    let leaf = Arc::new(AtomicU32::new(0));
    let l = Arc::clone(&leaf);

    let mut g = exec.add_graph();
    let r = g.vertex(item_with_triggers(
        |d| {
            d.interval(Duration::from_millis(10));
            Ok(())
        },
        |_| Ok(ControlFlow::StopChain),
    ));
    let leaf_v = g.vertex(item(move |_| {
        l.fetch_add(1, Ordering::SeqCst);
        Ok(ControlFlow::Continue)
    }));
    g.edge(r, leaf_v).root(r);
    g.build().unwrap();

    exec.run_n(1).unwrap();
    assert_eq!(leaf.load(Ordering::SeqCst), 0);
}

#[test]
fn vertex_err_stops_dispatch_and_propagates() {
    let mut exec = Executor::builder().worker_threads(2).build().unwrap();
    let mut g = exec.add_graph();
    let r = g.vertex(item_with_triggers(
        |d| {
            d.interval(Duration::from_millis(10));
            Ok(())
        },
        |_| Err(Box::new(std::io::Error::other("graph-err"))),
    ));
    let t = g.vertex(item(|_| Ok(ControlFlow::Continue)));
    g.edge(r, t).root(r);
    g.build().unwrap();

    let err = exec.run_n(1).expect_err("expected graph error");
    assert!(format!("{err}").contains("graph-err"));
}
