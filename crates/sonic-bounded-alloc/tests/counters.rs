//! TEST_0183 — allocation accounting API (REQ_0303).

#![allow(unsafe_code)]
#![allow(clippy::doc_markdown)]

use core::alloc::{GlobalAlloc, Layout};
use sonic_bounded_alloc::{BoundedAllocator, bounded_allocator};

static ALLOC: BoundedAllocator<8, 64, 1> = bounded_allocator!(8, 64);

#[test]
fn counters_reflect_allocation_history() {
    let layout = Layout::from_size_align(32, 8).unwrap();
    let alloc_start = ALLOC.alloc_count();
    let dealloc_start = ALLOC.dealloc_count();
    let peak_start = ALLOC.peak_blocks_used();

    // 1. Allocate 3 blocks.
    let p1 = unsafe { ALLOC.alloc(layout) };
    let p2 = unsafe { ALLOC.alloc(layout) };
    let p3 = unsafe { ALLOC.alloc(layout) };
    for p in [p1, p2, p3] {
        assert!(!p.is_null());
    }
    assert_eq!(ALLOC.alloc_count() - alloc_start, 3);
    assert_eq!(ALLOC.dealloc_count() - dealloc_start, 0);
    assert!(ALLOC.peak_blocks_used() >= peak_start.max(3));

    let peak_after_3 = ALLOC.peak_blocks_used();

    // 2. Free 1 block.
    unsafe { ALLOC.dealloc(p1, layout) };
    assert_eq!(ALLOC.alloc_count() - alloc_start, 3);
    assert_eq!(ALLOC.dealloc_count() - dealloc_start, 1);
    assert_eq!(ALLOC.peak_blocks_used(), peak_after_3);

    // 3. Allocate 2 more (now 4 live).
    let p4 = unsafe { ALLOC.alloc(layout) };
    let p5 = unsafe { ALLOC.alloc(layout) };
    assert!(!p4.is_null());
    assert!(!p5.is_null());
    assert_eq!(ALLOC.alloc_count() - alloc_start, 5);
    assert_eq!(ALLOC.dealloc_count() - dealloc_start, 1);
    assert!(ALLOC.peak_blocks_used() >= 4);

    // 4. Free everything.
    for p in [p2, p3, p4, p5] {
        unsafe { ALLOC.dealloc(p, layout) };
    }
    assert_eq!(ALLOC.alloc_count() - alloc_start, 5);
    assert_eq!(ALLOC.dealloc_count() - dealloc_start, 5);
    assert_eq!(ALLOC.live_blocks(), 0);
}
