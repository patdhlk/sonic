//! Zero-allocation dispatch — verification for REQ_0060.
//!
//! Uses `sonic_bounded_alloc::CountingAllocator` as the test
//! binary's `#[global_allocator]` so every thread's allocations
//! (WaitSet thread + pool workers) are counted. A differential
//! measurement (`run_n(big) - run_n(small)`) isolates per-iteration
//! steady-state allocations from the one-time setup that happens
//! at the top of `dispatch_loop` (WaitSet construction, trigger
//! attachment, iceoryx2 lazy init).

#![allow(missing_docs)]
#![allow(clippy::doc_markdown, clippy::cast_possible_wrap)]

use core::time::Duration;
use sonic_bounded_alloc::CountingAllocator;
use sonic_executor::{ControlFlow, Executor, item, item_with_triggers};

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator::new();

fn count_allocs<R>(f: impl FnOnce() -> R) -> (usize, R) {
    ALLOC.reset();
    ALLOC.set_tracking(true);
    let r = f();
    ALLOC.set_tracking(false);
    (ALLOC.alloc_count(), r)
}

// ── Trivial chain that performs no per-iteration work ──────────────────────

fn trivial_chain() -> Vec<Box<dyn sonic_executor::ExecutableItem>> {
    let head = item_with_triggers(
        |d| {
            d.interval(Duration::from_millis(1));
            Ok(())
        },
        |_| Ok(ControlFlow::Continue),
    );
    let mid = item(|_| Ok(ControlFlow::Continue));
    let tail = item(|_| Ok(ControlFlow::Continue));
    vec![Box::new(head), Box::new(mid), Box::new(tail)]
}

// ── Zero-allocation assertions ─────────────────────────────────────────────
//
// REQ_0060 prohibits heap allocations during **steady-state execution** —
// i.e. per-iteration of the dispatch loop. One-time setup performed by
// `dispatch_loop` (WaitSet construction, trigger attachment) is *not*
// steady-state, so we measure per-iteration allocation via a differential:
//
//   run_n(M) - run_n(N) = (M - N) * per_iter_alloc + 0
//
// for M > N and N large enough to absorb first-call lazy initialisation.

const ITERS_BIG: usize = 100;
const ITERS_SMALL: usize = 10;

/// Returns the average steady-state allocations per dispatch iteration.
fn per_iter_allocs(exec: &mut Executor) -> i64 {
    // Warm up to absorb any one-shot init that happens on first dispatch.
    exec.run_n(ITERS_SMALL).unwrap();
    let (a_small, ()) = count_allocs(|| exec.run_n(ITERS_SMALL).unwrap());
    let (a_big, ()) = count_allocs(|| exec.run_n(ITERS_BIG).unwrap());
    let diff = a_big as i64 - a_small as i64;
    let iters = (ITERS_BIG - ITERS_SMALL) as i64;
    // Round up so any fractional alloc per iter is detected.
    (diff + iters - 1) / iters
}

#[test]
fn dispatch_zero_alloc_single_thread_chain() {
    let mut exec = Executor::builder().worker_threads(0).build().unwrap();
    exec.add_chain(trivial_chain()).unwrap();

    let per_iter = per_iter_allocs(&mut exec);
    assert_eq!(
        per_iter, 0,
        "REQ_0060 violated: ~{per_iter} steady-state allocations per iteration (single-threaded chain)"
    );
}

#[test]
fn dispatch_zero_alloc_two_workers_chain() {
    let mut exec = Executor::builder().worker_threads(2).build().unwrap();
    exec.add_chain(trivial_chain()).unwrap();

    let per_iter = per_iter_allocs(&mut exec);
    assert_eq!(
        per_iter, 0,
        "REQ_0060 violated: ~{per_iter} steady-state allocations per iteration (2 worker threads, chain)"
    );
}

#[test]
fn dispatch_zero_alloc_graph_diamond_two_workers() {
    let mut exec = Executor::builder().worker_threads(2).build().unwrap();
    let mut g = exec.add_graph();
    let r = g.vertex(item_with_triggers(
        |d| {
            d.interval(Duration::from_millis(1));
            Ok(())
        },
        |_| Ok(ControlFlow::Continue),
    ));
    let l = g.vertex(item(|_| Ok(ControlFlow::Continue)));
    let rt = g.vertex(item(|_| Ok(ControlFlow::Continue)));
    let m = g.vertex(item(|_| Ok(ControlFlow::Continue)));
    g.edge(r, l).edge(r, rt).edge(l, m).edge(rt, m).root(r);
    g.build().unwrap();

    let per_iter = per_iter_allocs(&mut exec);
    assert_eq!(
        per_iter, 0,
        "REQ_0060 violated: ~{per_iter} steady-state allocations per iteration (graph diamond, 2 workers)"
    );
}

#[test]
fn dispatch_zero_alloc_single_thread_single_item() {
    let mut exec = Executor::builder().worker_threads(0).build().unwrap();
    let it = item_with_triggers(
        |d| {
            d.interval(Duration::from_millis(1));
            Ok(())
        },
        |_| Ok(ControlFlow::Continue),
    );
    exec.add(it).unwrap();

    let per_iter = per_iter_allocs(&mut exec);
    assert_eq!(
        per_iter, 0,
        "REQ_0060 violated: ~{per_iter} steady-state allocations per iteration (single-threaded, Single task)"
    );
}

// ── Negative case: harness must catch a deliberate per-iteration alloc ─────

#[test]
fn harness_catches_deliberate_allocation() {
    let mut exec = Executor::builder().worker_threads(0).build().unwrap();
    let head = item_with_triggers(
        |d| {
            d.interval(Duration::from_millis(1));
            Ok(())
        },
        |_| {
            // Deliberate per-iteration heap allocation.
            let v: Vec<u8> = vec![1, 2, 3];
            core::hint::black_box(&v);
            Ok(ControlFlow::Continue)
        },
    );
    exec.add(head).unwrap();

    exec.run_n(1).unwrap();

    let (allocs, ()) = count_allocs(|| exec.run_n(10).unwrap());
    assert!(
        allocs >= 10,
        "harness regression: counting allocator did not catch deliberate vec! allocations (saw {allocs})"
    );
}
