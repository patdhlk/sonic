//! TEST_0219 — `ChannelRegistry` has stable insertion-order
//! iteration and `iter()` does not allocate on the cycle hot path
//! (`REQ_0328`). Allocations are verified via
//! `sonic_bounded_alloc::CountingAllocator` registered as the test
//! binary's `#[global_allocator]`.
//!
//! All sub-cases run inside a single `#[test]` function so the
//! process-wide `CountingAllocator` measurement window is not
//! polluted by allocations from a concurrent test-runner thread
//! setting up another `#[test]` in this binary. A per-test Mutex
//! does not suffice because cargo's harness allocates per-test
//! buffers before the test body acquires the lock.

#![allow(
    clippy::doc_markdown,
    clippy::cast_possible_truncation,
    clippy::explicit_iter_loop
)]

use sonic_bounded_alloc::CountingAllocator;
use sonic_connector_ethercat::{ChannelBinding, ChannelRegistry, EthercatRouting, PdoDirection};

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator::new();

fn make_registry_with(n: usize) -> ChannelRegistry {
    let mut r = ChannelRegistry::with_capacity(n);
    for i in 0..n {
        // Use leaked &'static names so the Cow is cheap and doesn't
        // count toward per-cycle allocation accounting.
        let name: &'static str = Box::leak(format!("ch_{i}").into_boxed_str());
        let routing = EthercatRouting::new(i as u16, PdoDirection::Tx, 0, 16);
        r.register(name, routing, PdoDirection::Tx, ChannelBinding::Unbound);
    }
    r
}

#[test]
fn registry_invariants() {
    // Case 1: iteration order matches registration order.
    {
        let r = make_registry_with(8);
        let names: Vec<&str> = r.iter().map(|c| c.descriptor_name.as_ref()).collect();
        let expected: Vec<String> = (0..8).map(|i| format!("ch_{i}")).collect();
        assert_eq!(names, expected);
    }

    // Case 2: handles index into the registry.
    {
        let mut r = ChannelRegistry::with_capacity(4);
        let h0 = r.register(
            "first",
            EthercatRouting::new(1, PdoDirection::Tx, 0, 8),
            PdoDirection::Tx,
            ChannelBinding::Unbound,
        );
        let h1 = r.register(
            "second",
            EthercatRouting::new(2, PdoDirection::Rx, 8, 8),
            PdoDirection::Rx,
            ChannelBinding::Unbound,
        );
        assert_eq!(r.get(h0).unwrap().descriptor_name, "first");
        assert_eq!(r.get(h1).unwrap().descriptor_name, "second");
    }

    // Case 3: empty registry iter is empty and alloc-free.
    {
        let r = ChannelRegistry::new();
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);

        ALLOC.reset();
        ALLOC.set_tracking(true);
        let count = r.iter().count();
        ALLOC.set_tracking(false);
        assert_eq!(count, 0);
        assert_eq!(ALLOC.alloc_count(), 0);
    }

    // Case 4 — TEST_0219 proper: 1000-cycle iteration must not
    // allocate. Build the registry BEFORE enabling tracking
    // (registration legitimately allocates).
    {
        let r = make_registry_with(16);
        // Warm the iteration once outside the tracking window so any
        // first-call quirks don't count.
        let _ = r.iter().count();

        ALLOC.reset();
        ALLOC.set_tracking(true);
        let mut total = 0_u64;
        for _ in 0..1_000 {
            for channel in r.iter() {
                // Read a field to prevent the optimiser from eliding
                // the loop entirely.
                total = total.wrapping_add(u64::from(channel.routing.bit_length));
            }
        }
        ALLOC.set_tracking(false);

        let allocs = ALLOC.alloc_count();
        assert_eq!(
            allocs, 0,
            "iter() allocated {allocs} times across 1000 cycles × 16 channels — REQ_0328 prohibits per-cycle alloc"
        );
        // Anti-elision check: total should be 16_000 (16 channels ×
        // 16 bit_length × 1000 cycles) — silences "loop never ran"
        // worries.
        assert_eq!(total, 16 * 16 * 1000);
    }
}
