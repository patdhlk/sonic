//! Sample binary demonstrating fail-closed-on-cap behaviour
//! (REQ_0301). Registers a `BoundedAllocator<MAX_BLOCKS, 4096>` as
//! `#[global_allocator]`. The arena is sized so that:
//!
//! * Rust runtime + stdlib startup fits inside the first few
//!   blocks (Rust's stdlib allocates a few hundred bytes before
//!   `main` runs).
//! * The demo loop then `Box::new`s steadily-growing allocations
//!   until the cap is hit; the over-cap allocation returns null,
//!   Rust's default `alloc_error_handler` aborts the process.
//!
//! Run with:
//!
//! ```sh
//! cargo run -p sonic-bounded-alloc --example fail_closed
//! echo $?   # expect 134 (SIGABRT) or similar non-zero
//! ```
//!
//! Expected output: a handful of "iter N: allocated; …" lines
//! showing live block count climbing toward `MAX_BLOCKS`, then
//! `memory allocation of N bytes failed` followed by abort.

#![allow(missing_docs)]

use sonic_bounded_alloc::declare_global_allocator;

const MAX_BLOCKS: usize = 16;
const BLOCK_SIZE: usize = 4096;

declare_global_allocator!(ALLOC, MAX_BLOCKS, BLOCK_SIZE);

fn main() {
    println!("sonic-bounded-alloc fail_closed demo");
    println!("arena: {MAX_BLOCKS} blocks × {BLOCK_SIZE} bytes");
    println!(
        "blocks already in use after Rust runtime startup: {}",
        ALLOC.live_blocks()
    );
    println!();

    let mut held: Vec<Box<[u8; 1024]>> = Vec::new();
    for i in 0..64 {
        let b: Box<[u8; 1024]> = Box::new([(i & 0xff) as u8; 1024]);
        held.push(b);
        println!(
            "iter {i}: alloc_count={}, live={}, peak={}",
            ALLOC.alloc_count(),
            ALLOC.live_blocks(),
            ALLOC.peak_blocks_used()
        );
    }

    // Unreachable: an allocation past the cap returns null, and
    // Rust's default alloc_error_handler aborts the process.
    println!("unexpectedly reached end of main (cap was not exhausted)");
}
