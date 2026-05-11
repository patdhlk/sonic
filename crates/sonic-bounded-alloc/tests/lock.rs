//! TEST_0182 — lock() then alloc panics (REQ_0302).
//!
//! NB: this test crate is built with the workspace default unwind
//! panic strategy so cargo-test can catch the panic via
//! `#[should_panic]`. Deployment binaries that rely on REQ_0302 to
//! abort the process must set `panic = "abort"` themselves.

#![allow(unsafe_code)]
#![allow(clippy::doc_markdown)]

use core::alloc::{GlobalAlloc, Layout};
use sonic_bounded_alloc::{BoundedAllocator, bounded_allocator};

static UNLOCKED_ALLOC: BoundedAllocator<4, 32, 1> = bounded_allocator!(4, 32);

#[test]
fn alloc_works_before_lock() {
    let layout = Layout::from_size_align(16, 8).unwrap();
    let p = unsafe { UNLOCKED_ALLOC.alloc(layout) };
    assert!(!p.is_null());
    assert!(!UNLOCKED_ALLOC.is_locked());
    unsafe { UNLOCKED_ALLOC.dealloc(p, layout) };
}

static LOCKED_ALLOC: BoundedAllocator<4, 32, 1> = bounded_allocator!(4, 32);

#[test]
#[should_panic(expected = "allocation attempted after lock()")]
fn alloc_after_lock_panics() {
    let layout = Layout::from_size_align(16, 8).unwrap();
    // One allocation pre-lock is fine.
    let p = unsafe { LOCKED_ALLOC.alloc(layout) };
    assert!(!p.is_null());

    LOCKED_ALLOC.lock();
    assert!(LOCKED_ALLOC.is_locked());

    // Post-lock allocation must panic.
    let _ = unsafe { LOCKED_ALLOC.alloc(layout) };
}
