//! Static pre-allocated bounded `#[global_allocator]` with hard caps
//! on per-allocation size and total live block count.
//!
//! See `FEAT_0040` and `REQ_0300..REQ_0304` for the design contract.
//!
//! # Quick start
//!
//! ```ignore
//! use sonic_bounded_alloc::declare_global_allocator;
//!
//! // Declares a `#[global_allocator]` static named `ALLOC` with a
//! // 512 × 1024-byte arena. Maximum per-allocation size is 1024
//! // bytes; maximum live blocks is 512.
//! declare_global_allocator!(ALLOC, 512, 1024);
//!
//! fn main() {
//!     let s = String::from("hello");
//!     assert_eq!(ALLOC.alloc_count(), 1);
//!     drop(s);
//!     assert_eq!(ALLOC.dealloc_count(), 1);
//! }
//! ```
//!
//! # Lock-after-init
//!
//! Call `ALLOC.lock()` once the program reaches its steady-state
//! point (e.g. after `Executor::build`). Any subsequent allocation
//! call panics — which under `panic = "abort"` aborts the process,
//! catching stray heap activity that escaped review.
//!
//! ```ignore
//! ALLOC.lock();
//! let _ = String::from("this aborts the process");
//! ```
//!
//! The consuming binary's `Cargo.toml` must set
//! `[profile.release] panic = "abort"` (and `[profile.dev]` if dev
//! builds also need the guarantee) — otherwise the panic itself
//! will allocate the unwinder's payload string.

#![no_std]
#![deny(unsafe_code)]
#![warn(missing_docs)]

#[cfg(feature = "std")]
extern crate std;

use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

// ── Block ──────────────────────────────────────────────────────────────────

/// One arena block. `align(64)` is the maximum `layout.align()` the
/// allocator can serve; allocations whose `Layout::align()` exceeds
/// 64 are rejected with `null`.
///
/// `BLOCK_SIZE` should be a multiple of 64 to avoid intra-block
/// padding that would inflate the arena's footprint beyond
/// `MAX_BLOCKS * BLOCK_SIZE`. Smaller values still work — they
/// just waste a few bytes per block.
#[repr(C, align(64))]
#[doc(hidden)]
pub struct Block<const N: usize>(UnsafeCell<[u8; N]>);

impl<const N: usize> Block<N> {
    #[doc(hidden)]
    #[must_use]
    pub const fn new() -> Self {
        Self(UnsafeCell::new([0; N]))
    }

    #[doc(hidden)]
    #[must_use]
    pub fn as_mut_ptr(&self) -> *mut u8 {
        self.0.get().cast::<u8>()
    }
}

impl<const N: usize> Default for Block<N> {
    fn default() -> Self {
        Self::new()
    }
}

// SAFETY: blocks are accessed via the bitmap's compare-exchange
// protocol — only the thread that flipped a bit `1 -> 0` holds a
// logical `&mut` to the corresponding block, and only until it flips
// the bit back to `1` in `dealloc`. No aliasing across threads, so
// `Sync` is sound.
#[allow(unsafe_code)]
unsafe impl<const N: usize> Sync for Block<N> {}

// ── BoundedAllocator ───────────────────────────────────────────────────────

/// Static pre-allocated bounded global allocator.
///
/// Generic parameters
///
/// * `MAX_BLOCKS` — maximum number of simultaneously-live blocks.
/// * `BLOCK_SIZE` — maximum bytes per allocation. Should be a
///   multiple of 64 to avoid intra-block padding.
/// * `BITMAP_WORDS` — must be `(MAX_BLOCKS + 63) / 64`. Use the
///   [`declare_global_allocator!`] / [`declare_bounded_allocator!`]
///   macros to compute it automatically.
pub struct BoundedAllocator<
    const MAX_BLOCKS: usize,
    const BLOCK_SIZE: usize,
    const BITMAP_WORDS: usize,
> {
    /// The arena. Each block is `align(64)` and `BLOCK_SIZE` bytes
    /// (rounded up to 64 if smaller). Total footprint ≈
    /// `MAX_BLOCKS * BLOCK_SIZE` for BLOCK_SIZE ≥ 64.
    arena: [Block<BLOCK_SIZE>; MAX_BLOCKS],
    /// Per-block free flag. Bit `i` of word `w` represents block
    /// `w * 64 + i`. `1` = free, `0` = in use. Words past
    /// `MAX_BLOCKS / 64` may have bogus "free" bits in their tail —
    /// `try_claim_block` filters those out by checking the derived
    /// block index against `MAX_BLOCKS`.
    bitmap: [AtomicU64; BITMAP_WORDS],
    /// Running count of successful allocations since process start.
    alloc_count: AtomicUsize,
    /// Running count of `dealloc` calls since process start.
    dealloc_count: AtomicUsize,
    /// High-water mark of simultaneously-live blocks.
    peak_in_use: AtomicUsize,
    /// Live block count (alloc_count - dealloc_count, atomically).
    in_use: AtomicUsize,
    /// Lock flag. When `true`, every `alloc` panics — required by
    /// REQ_0302. One-way; no `unlock` method exists.
    locked: AtomicBool,
}

// SAFETY: a global allocator must be `Sync`. Every field above is
// independently `Sync` (atomics + `Block`'s manual `Sync` impl),
// which is sufficient.

impl<const MAX_BLOCKS: usize, const BLOCK_SIZE: usize, const BITMAP_WORDS: usize>
    BoundedAllocator<MAX_BLOCKS, BLOCK_SIZE, BITMAP_WORDS>
{
    /// Construct a fresh allocator. Intended for use as a `static`
    /// initialiser.
    ///
    /// All bitmap words are initialised to all-ones, meaning every
    /// block is free. The few tail bits past `MAX_BLOCKS` in the
    /// last word are also set, but the bit-scan refuses to allocate
    /// them (block index >= MAX_BLOCKS rejection).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            arena: [const { Block::new() }; MAX_BLOCKS],
            bitmap: [const { AtomicU64::new(u64::MAX) }; BITMAP_WORDS],
            alloc_count: AtomicUsize::new(0),
            dealloc_count: AtomicUsize::new(0),
            peak_in_use: AtomicUsize::new(0),
            in_use: AtomicUsize::new(0),
            locked: AtomicBool::new(false),
        }
    }

    /// Engage lock-after-init mode. Every subsequent `alloc` call
    /// panics immediately. One-way: there is no `unlock` method.
    pub fn lock(&self) {
        self.locked.store(true, Ordering::Release);
    }

    /// Is the allocator locked? See [`Self::lock`].
    #[must_use]
    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Acquire)
    }

    /// Total successful `alloc` calls since process start.
    #[must_use]
    pub fn alloc_count(&self) -> usize {
        self.alloc_count.load(Ordering::Relaxed)
    }

    /// Total `dealloc` calls since process start.
    #[must_use]
    pub fn dealloc_count(&self) -> usize {
        self.dealloc_count.load(Ordering::Relaxed)
    }

    /// High-water mark of simultaneously-live blocks.
    #[must_use]
    pub fn peak_blocks_used(&self) -> usize {
        self.peak_in_use.load(Ordering::Relaxed)
    }

    /// Currently-live block count.
    #[must_use]
    pub fn live_blocks(&self) -> usize {
        self.in_use.load(Ordering::Relaxed)
    }

    /// Total arena bytes addressed by this allocator.
    #[must_use]
    pub const fn capacity_bytes(&self) -> usize {
        MAX_BLOCKS * BLOCK_SIZE
    }

    /// Try to claim a free block. Returns its index (0..MAX_BLOCKS)
    /// or `None` if the arena is fully allocated.
    fn try_claim_block(&self) -> Option<usize> {
        for word_idx in 0..BITMAP_WORDS {
            loop {
                let word = self.bitmap[word_idx].load(Ordering::Acquire);
                if word == 0 {
                    // No free bits in this word.
                    break;
                }
                let bit_idx = word.trailing_zeros() as usize;
                let block_idx = word_idx * 64 + bit_idx;
                if block_idx >= MAX_BLOCKS {
                    // Bogus tail bit — fall through to the next word.
                    break;
                }
                let mask = 1_u64 << bit_idx;
                let new_word = word & !mask;
                match self.bitmap[word_idx].compare_exchange(
                    word,
                    new_word,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => return Some(block_idx),
                    Err(_) => {
                        // Lost the race; retry within this word.
                        continue;
                    }
                }
            }
        }
        None
    }

    /// Release block `block_idx` back to the free pool.
    fn release_block(&self, block_idx: usize) {
        debug_assert!(block_idx < MAX_BLOCKS);
        let word_idx = block_idx / 64;
        let bit_idx = block_idx % 64;
        let mask = 1_u64 << bit_idx;
        self.bitmap[word_idx].fetch_or(mask, Ordering::AcqRel);
    }

    /// Compute the block index for a pointer returned by a previous
    /// `alloc`. The pointer must lie inside `self.arena`.
    fn block_index_of(&self, ptr: *mut u8) -> usize {
        let stride = core::mem::size_of::<Block<BLOCK_SIZE>>();
        let arena_base = self.arena.as_ptr().cast::<u8>() as usize;
        let offset = (ptr as usize).wrapping_sub(arena_base);
        offset / stride
    }
}

impl<const MAX_BLOCKS: usize, const BLOCK_SIZE: usize, const BITMAP_WORDS: usize> Default
    for BoundedAllocator<MAX_BLOCKS, BLOCK_SIZE, BITMAP_WORDS>
{
    fn default() -> Self {
        Self::new()
    }
}

// SAFETY: `unsafe impl GlobalAlloc` is the trait's requirement; the
// implementation upholds the trait contract (returns a pointer to a
// region of at least `layout.size()` bytes, aligned to
// `layout.align()`, or null on failure). Aliasing is prevented by
// the bitmap CAS — a thread observing a `1 -> 0` transition on a
// bit is the unique owner of the corresponding block until it CASs
// the bit back to `1` in `dealloc`.
#[allow(unsafe_code)]
unsafe impl<
    const MAX_BLOCKS: usize,
    const BLOCK_SIZE: usize,
    const BITMAP_WORDS: usize,
> GlobalAlloc for BoundedAllocator<MAX_BLOCKS, BLOCK_SIZE, BITMAP_WORDS>
{
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if self.is_locked() {
            panic!(
                "sonic-bounded-alloc: allocation attempted after lock() (REQ_0302); \
                 sized {} bytes, alignment {}",
                layout.size(),
                layout.align()
            );
        }
        if layout.size() > BLOCK_SIZE {
            return core::ptr::null_mut();
        }
        // Block alignment is 64 (from `repr(C, align(64))` on `Block`).
        if layout.align() > 64 {
            return core::ptr::null_mut();
        }
        let Some(block_idx) = self.try_claim_block() else {
            return core::ptr::null_mut();
        };
        let ptr = self.arena[block_idx].as_mut_ptr();
        self.alloc_count.fetch_add(1, Ordering::Relaxed);
        let in_use_after = self.in_use.fetch_add(1, Ordering::Relaxed) + 1;
        // Update high-water mark monotonically.
        let mut peak = self.peak_in_use.load(Ordering::Relaxed);
        while in_use_after > peak {
            match self.peak_in_use.compare_exchange_weak(
                peak,
                in_use_after,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(observed) => peak = observed,
            }
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        let block_idx = self.block_index_of(ptr);
        self.release_block(block_idx);
        self.dealloc_count.fetch_add(1, Ordering::Relaxed);
        self.in_use.fetch_sub(1, Ordering::Relaxed);
    }
}

// ── Convenience macros ────────────────────────────────────────────────────

/// Declare a `static BoundedAllocator` registered as
/// `#[global_allocator]`, computing the bitmap word count from
/// `MAX_BLOCKS` automatically.
///
/// # Example
///
/// ```ignore
/// sonic_bounded_alloc::declare_global_allocator!(ALLOC, 512, 1024);
/// ```
///
/// Expands to roughly:
///
/// ```ignore
/// #[global_allocator]
/// static ALLOC: sonic_bounded_alloc::BoundedAllocator<512, 1024, 8> =
///     sonic_bounded_alloc::BoundedAllocator::new();
/// ```
#[macro_export]
macro_rules! declare_global_allocator {
    ($name:ident, $max_blocks:expr, $block_size:expr $(,)?) => {
        #[global_allocator]
        static $name: $crate::BoundedAllocator<
            { $max_blocks },
            { $block_size },
            { ($max_blocks + 63) / 64 },
        > = $crate::BoundedAllocator::new();
    };
}

/// Construct a const-evaluable `BoundedAllocator` expression with
/// the bitmap word count computed automatically. Useful in
/// regular (non-global-allocator) `static` declarations and tests.
///
/// ```ignore
/// static TEST_ALLOC: BoundedAllocator<4, 32, 1> =
///     sonic_bounded_alloc::bounded_allocator!(4, 32);
/// ```
#[macro_export]
macro_rules! bounded_allocator {
    ($max_blocks:expr, $block_size:expr $(,)?) => {
        $crate::BoundedAllocator::<
            { $max_blocks },
            { $block_size },
            { ($max_blocks + 63) / 64 },
        >::new()
    };
}

// ── CountingAllocator (std-only) ───────────────────────────────────────────

#[cfg(feature = "std")]
pub use self::counting::CountingAllocator;

#[cfg(feature = "std")]
mod counting {
    //! Unbounded counter wrapper around `std::alloc::System`.
    //!
    //! Distinct from `BoundedAllocator` — `CountingAllocator` does
    //! **not** enforce caps; it delegates every allocation to the
    //! system allocator and only counts. Intended for test
    //! harnesses that need cross-thread alloc/dealloc accounting
    //! without restricting heap size.

    use core::alloc::{GlobalAlloc, Layout};
    use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::alloc::System;

    /// `#[global_allocator]`-compatible wrapper around the system
    /// allocator that counts every successful allocation and
    /// deallocation while a thread-shared `TRACKING` flag is set.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use sonic_bounded_alloc::CountingAllocator;
    ///
    /// #[global_allocator]
    /// static A: CountingAllocator = CountingAllocator::new();
    ///
    /// fn main() {
    ///     A.reset();
    ///     A.set_tracking(true);
    ///     // ... measure a region of interest ...
    ///     A.set_tracking(false);
    ///     println!("allocs in region: {}", A.alloc_count());
    /// }
    /// ```
    pub struct CountingAllocator {
        alloc_count: AtomicUsize,
        dealloc_count: AtomicUsize,
        tracking: AtomicBool,
    }

    impl CountingAllocator {
        /// Construct a new counter. Suitable for use in a `static`.
        #[must_use]
        pub const fn new() -> Self {
            Self {
                alloc_count: AtomicUsize::new(0),
                dealloc_count: AtomicUsize::new(0),
                tracking: AtomicBool::new(false),
            }
        }

        /// Enable or disable counting. When `false`, allocations
        /// pass through without incrementing the counters.
        pub fn set_tracking(&self, on: bool) {
            self.tracking.store(on, Ordering::Release);
        }

        /// Is counting currently enabled?
        #[must_use]
        pub fn is_tracking(&self) -> bool {
            self.tracking.load(Ordering::Acquire)
        }

        /// Reset both counters to zero.
        pub fn reset(&self) {
            self.alloc_count.store(0, Ordering::Relaxed);
            self.dealloc_count.store(0, Ordering::Relaxed);
        }

        /// Total successful allocations counted since the most
        /// recent `reset`.
        #[must_use]
        pub fn alloc_count(&self) -> usize {
            self.alloc_count.load(Ordering::Relaxed)
        }

        /// Total deallocations counted since the most recent `reset`.
        #[must_use]
        pub fn dealloc_count(&self) -> usize {
            self.dealloc_count.load(Ordering::Relaxed)
        }
    }

    impl Default for CountingAllocator {
        fn default() -> Self {
            Self::new()
        }
    }

    // SAFETY: delegates to the system allocator; the wrapper only
    // adds atomic counter increments, which are themselves
    // thread-safe.
    #[allow(unsafe_code)]
    unsafe impl GlobalAlloc for CountingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            if self.tracking.load(Ordering::Relaxed) {
                self.alloc_count.fetch_add(1, Ordering::Relaxed);
            }
            // SAFETY: forwarding the caller's contract unchanged.
            unsafe { System.alloc(layout) }
        }
        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            if self.tracking.load(Ordering::Relaxed) {
                self.alloc_count.fetch_add(1, Ordering::Relaxed);
            }
            // SAFETY: forwarding the caller's contract unchanged.
            unsafe { System.alloc_zeroed(layout) }
        }
        unsafe fn realloc(
            &self,
            ptr: *mut u8,
            layout: Layout,
            new_size: usize,
        ) -> *mut u8 {
            if self.tracking.load(Ordering::Relaxed) {
                self.alloc_count.fetch_add(1, Ordering::Relaxed);
            }
            // SAFETY: ptr/layout pair previously returned by alloc;
            // forwarding unchanged.
            unsafe { System.realloc(ptr, layout, new_size) }
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            if self.tracking.load(Ordering::Relaxed) {
                self.dealloc_count.fetch_add(1, Ordering::Relaxed);
            }
            // SAFETY: ptr/layout pair previously returned by alloc.
            unsafe { System.dealloc(ptr, layout) }
        }
    }
}
