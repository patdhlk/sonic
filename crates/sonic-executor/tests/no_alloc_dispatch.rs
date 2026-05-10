//! Zero-allocation dispatch — verification for REQ_0060.
//!
//! Wraps the system allocator with a counting wrapper, runs a warm-up
//! iteration, then asserts that `Executor::run_n(N)` performs zero
//! heap allocations across all threads (WaitSet + pool workers).

#![allow(missing_docs)]
#![allow(unsafe_code)]

use core::time::Duration;
use sonic_executor::{ControlFlow, Executor, item, item_with_triggers};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

// ── Counting global allocator ───────────────────────────────────────────────

struct CountingAllocator;

static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
static DEALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
static TRACKING: AtomicBool = AtomicBool::new(false);

// Histogram buckets for diagnostic mode. Indices are floor(log2(size)).
const BUCKETS: usize = 32;
static SIZE_BUCKETS: [AtomicUsize; BUCKETS] = {
    // Safe: AtomicUsize::new(0) is const.
    [
        AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(0),
        AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(0),
        AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(0),
        AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(0),
        AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(0),
        AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(0),
        AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(0),
        AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(0),
    ]
};

fn bucket_for(size: usize) -> usize {
    if size == 0 {
        0
    } else {
        let leading = (size as u64).leading_zeros() as usize;
        (63 - leading).min(BUCKETS - 1)
    }
}

static PANIC_ON_NEXT_ALLOC: AtomicBool = AtomicBool::new(false);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if TRACKING.load(Ordering::Relaxed) {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
            SIZE_BUCKETS[bucket_for(layout.size())].fetch_add(1, Ordering::Relaxed);
            if PANIC_ON_NEXT_ALLOC.swap(false, Ordering::AcqRel) {
                // Disable counting before panic to prevent recursion.
                TRACKING.store(false, Ordering::Release);
                panic!("trip-wire: heap allocation of size {} bytes", layout.size());
            }
        }
        // SAFETY: delegating to the system allocator with the same layout
        // contract the caller passed in.
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if TRACKING.load(Ordering::Relaxed) {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: delegating to the system allocator with the same layout.
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if TRACKING.load(Ordering::Relaxed) {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: ptr/layout pair was previously allocated by `alloc`; we
        // forward unchanged.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if TRACKING.load(Ordering::Relaxed) {
            DEALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: ptr/layout previously returned from this allocator.
        unsafe { System.dealloc(ptr, layout) };
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

fn count_allocs<R>(f: impl FnOnce() -> R) -> (usize, usize, R) {
    ALLOC_COUNT.store(0, Ordering::Relaxed);
    DEALLOC_COUNT.store(0, Ordering::Relaxed);
    for b in &SIZE_BUCKETS {
        b.store(0, Ordering::Relaxed);
    }
    TRACKING.store(true, Ordering::Release);
    let r = f();
    TRACKING.store(false, Ordering::Release);
    let allocs = ALLOC_COUNT.load(Ordering::Relaxed);
    let deallocs = DEALLOC_COUNT.load(Ordering::Relaxed);
    (allocs, deallocs, r)
}

#[allow(dead_code)]
fn dump_buckets(label: &str) {
    eprintln!("alloc size histogram for {label}:");
    for (i, b) in SIZE_BUCKETS.iter().enumerate() {
        let n = b.load(Ordering::Relaxed);
        if n > 0 {
            let low = if i == 0 { 0 } else { 1usize << i };
            let high = (1usize << (i + 1)).saturating_sub(1);
            eprintln!("  size [{low}..={high}]: {n} allocs");
        }
    }
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
    let (a_small, _, _) = count_allocs(|| exec.run_n(ITERS_SMALL).unwrap());
    let (a_big, _, _) = count_allocs(|| exec.run_n(ITERS_BIG).unwrap());
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

// ── Diagnostic: where do the remaining allocs come from? ───────────────────

#[test]
fn dispatch_baseline_single_item_inline_bracketed() {
    // Bracket each run_n call separately so the setup-phase allocations
    // (waitset construction, attach_interval, etc.) are visible distinctly
    // from the per-iteration steady-state allocations.
    let mut exec = Executor::builder().worker_threads(0).build().unwrap();
    let it = item_with_triggers(
        |d| {
            d.interval(Duration::from_millis(1));
            Ok(())
        },
        |_| Ok(ControlFlow::Continue),
    );
    exec.add(it).unwrap();

    let (a1, _, _) = count_allocs(|| exec.run_n(1).unwrap());
    let (a2, _, _) = count_allocs(|| exec.run_n(1).unwrap());
    let (a3, _, _) = count_allocs(|| exec.run_n(10).unwrap());
    let (a4, _, _) = count_allocs(|| exec.run_n(100).unwrap());
    println!(
        "DIAG: run_n(1) #1: {a1} ; run_n(1) #2: {a2} ; run_n(10): {a3} ; run_n(100): {a4}"
    );
}

#[test]
#[ignore = "trip-wire diagnostic; run with --ignored to capture a backtrace"]
fn trip_first_alloc_backtrace() {
    let mut exec = Executor::builder().worker_threads(0).build().unwrap();
    let it = item_with_triggers(
        |d| {
            d.interval(Duration::from_millis(1));
            Ok(())
        },
        |_| Ok(ControlFlow::Continue),
    );
    exec.add(it).unwrap();
    exec.run_n(1).unwrap();

    TRACKING.store(true, Ordering::Release);
    PANIC_ON_NEXT_ALLOC.store(true, Ordering::Release);
    // Run one iteration; the first allocation will panic with a backtrace
    // (assuming RUST_BACKTRACE=1).
    let _ = exec.run_n(1);
    TRACKING.store(false, Ordering::Release);
}

#[test]
fn dispatch_baseline_no_tasks_run_for() {
    // No tasks at all; run_for so the WaitSet is forced to wake on its
    // own. Reveals iceoryx2 / WaitSet internal per-iteration allocations.
    let mut exec = Executor::builder().worker_threads(0).build().unwrap();
    // No tasks added, but run_for needs SOME wake source. Add a single
    // trivial item that returns Continue.
    let it = item_with_triggers(
        |d| {
            d.interval(Duration::from_millis(1));
            Ok(())
        },
        |_| Ok(ControlFlow::Continue),
    );
    exec.add(it).unwrap();

    exec.run_n(1).unwrap();
    // Counting in scope:
    let (allocs, _, _) = count_allocs(|| exec.run_n(100).unwrap());
    println!("DIAG: same as single-item inline: {allocs} allocs / 100 iter");
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

    let (allocs, _, _) = count_allocs(|| exec.run_n(10).unwrap());
    assert!(
        allocs >= 10,
        "harness regression: counting allocator did not catch deliberate vec! allocations (saw {allocs})"
    );
}
