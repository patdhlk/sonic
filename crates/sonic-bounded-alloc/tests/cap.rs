//! TEST_0180 / TEST_0181 — cap exhaustion, oversize, and steady-state
//! cap behaviour.

#![allow(unsafe_code)]
#![allow(clippy::doc_markdown)]

use core::alloc::{GlobalAlloc, Layout};
use sonic_bounded_alloc::{BoundedAllocator, bounded_allocator};

static ALLOC: BoundedAllocator<4, 32, 1> = bounded_allocator!(4, 32);

#[test]
fn cap_exhaustion_and_recovery() {
    let layout = Layout::from_size_align(16, 8).unwrap();
    let mut pointers = [core::ptr::null_mut::<u8>(); 4];
    for slot in &mut pointers {
        // SAFETY: alloc returns a pointer to an exclusive block.
        let p = unsafe { ALLOC.alloc(layout) };
        assert!(!p.is_null(), "first four allocations must succeed");
        *slot = p;
    }
    // Fifth allocation must fail-closed (REQ_0301).
    let p5 = unsafe { ALLOC.alloc(layout) };
    assert!(p5.is_null(), "5th allocation must return null (cap = 4)");

    // Free one and re-allocate.
    unsafe { ALLOC.dealloc(pointers[0], layout) };
    let p_re = unsafe { ALLOC.alloc(layout) };
    assert!(!p_re.is_null(), "freed slot must be reusable");
    pointers[0] = p_re;

    // Cleanup.
    for p in pointers {
        unsafe { ALLOC.dealloc(p, layout) };
    }
}

#[test]
fn oversize_request_returns_null() {
    let big = Layout::from_size_align(64, 8).unwrap();
    let p = unsafe { ALLOC.alloc(big) };
    assert!(p.is_null(), "size > BLOCK_SIZE must return null");
}

#[test]
fn excessive_alignment_returns_null() {
    let aligned = Layout::from_size_align(8, 128).unwrap();
    let p = unsafe { ALLOC.alloc(aligned) };
    assert!(p.is_null(), "align > 64 must return null");
}

static BURST_ALLOC: BoundedAllocator<8, 64, 1> = bounded_allocator!(8, 64);

#[test]
fn balanced_alloc_dealloc_burst_recovers_all_capacity() {
    let layout = Layout::from_size_align(32, 8).unwrap();
    for _ in 0..10_000 {
        // SAFETY: same exclusive-block contract as above.
        let p = unsafe { BURST_ALLOC.alloc(layout) };
        assert!(!p.is_null());
        unsafe { BURST_ALLOC.dealloc(p, layout) };
    }

    // After burst, all 8 blocks must be allocatable again.
    let mut held = [core::ptr::null_mut::<u8>(); 8];
    for slot in &mut held {
        let p = unsafe { BURST_ALLOC.alloc(layout) };
        assert!(!p.is_null(), "all 8 blocks must be reusable after burst");
        *slot = p;
    }
    // 9th must fail.
    assert!(unsafe { BURST_ALLOC.alloc(layout) }.is_null());
    for p in held {
        unsafe { BURST_ALLOC.dealloc(p, layout) };
    }
}
