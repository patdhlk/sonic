//! TEST_0184 — concurrent alloc/dealloc safety (REQ_0304).

#![allow(unsafe_code)]
#![allow(clippy::doc_markdown, clippy::significant_drop_tightening)]

use core::alloc::{GlobalAlloc, Layout};
use sonic_bounded_alloc::{BoundedAllocator, bounded_allocator};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

static ALLOC: BoundedAllocator<256, 64, 4> = bounded_allocator!(256, 64);

const THREADS: usize = 4;
const ITERS_PER_THREAD: usize = 1000;

#[test]
fn concurrent_alloc_dealloc_no_double_allocation() {
    let layout = Layout::from_size_align(32, 8).unwrap();
    let in_flight: Arc<Mutex<HashSet<usize>>> = Arc::new(Mutex::new(HashSet::new()));

    let mut handles = Vec::with_capacity(THREADS);
    for _ in 0..THREADS {
        let in_flight = Arc::clone(&in_flight);
        let h = thread::spawn(move || {
            for _ in 0..ITERS_PER_THREAD {
                // SAFETY: GlobalAlloc trait — caller invariants
                // satisfied (layout valid, ptr matched on dealloc).
                let p = loop {
                    let p = unsafe { ALLOC.alloc(layout) };
                    if !p.is_null() {
                        break p;
                    }
                    // Arena momentarily full — yield and retry.
                    thread::yield_now();
                };
                {
                    // Claim the pointer; no other thread may have
                    // observed the same address simultaneously.
                    let mut s = in_flight.lock().unwrap();
                    assert!(
                        s.insert(p as usize),
                        "double-allocation detected: ptr {p:p} already in flight"
                    );
                }
                // Hold the block briefly so the arena reaches a
                // meaningfully non-empty state.
                thread::sleep(Duration::from_nanos(50));
                {
                    let mut s = in_flight.lock().unwrap();
                    assert!(
                        s.remove(&(p as usize)),
                        "ptr {p:p} freed but wasn't recorded"
                    );
                }
                unsafe { ALLOC.dealloc(p, layout) };
            }
        });
        handles.push(h);
    }
    for h in handles {
        h.join().unwrap();
    }

    assert!(in_flight.lock().unwrap().is_empty());
    // After every alloc was matched with a dealloc, the live count
    // returns to whatever pre-existing value (or zero on a fresh
    // process).
    assert_eq!(ALLOC.live_blocks(), 0);
}
