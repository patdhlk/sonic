//! Hand-rolled bounded MPSC ring used by `Graph::run_once_borrowed` to
//! communicate "vertex `j` became ready" from completed pool workers
//! (producers) back to the WaitSet thread (consumer).
//!
//! Required by REQ_0060 — the previous `crossbeam_channel::unbounded`
//! design allocated internal Arc'd shared state on every
//! `Graph::run_once` call. The ring here is allocated **once** by
//! `ReadyRing::new` at graph-finish time and reused across every
//! dispatch iteration via `ReadyRing::reset`.
//!
//! Capacity is rounded up to the next power of two so wraparound is a
//! cheap mask. Producers CAS the tail to reserve a slot, then store the
//! value; the consumer spins briefly on a SENTINEL value when a slot
//! has been reserved but not yet filled (the producer is between its
//! tail-CAS and its value-store). The spin is bounded by single-store
//! latency on modern memory systems.

#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};

/// Sentinel placed in empty slots. `usize::MAX` is reserved by the
/// graph dispatcher to mean "cancelled vertex" — that's fine because
/// the cancellation path uses `counters[i].swap(usize::MAX, ...)` on a
/// different array; the ready ring carries vertex indices in the range
/// `0..n_vertices` and never legitimately holds `usize::MAX`.
const SENTINEL: usize = usize::MAX;

/// Bounded multi-producer single-consumer ring buffer. Performs **one**
/// heap allocation at construction and **none** thereafter.
pub(crate) struct ReadyRing {
    buf: Box<[AtomicUsize]>,
    mask: usize,
    head: AtomicUsize,
    tail: AtomicUsize,
}

impl ReadyRing {
    /// Allocate a ring with capacity at least `min_capacity`, rounded
    /// up to the next power of two (and at least 2). One-time
    /// allocation; called from `Graph::finish`.
    pub(crate) fn new(min_capacity: usize) -> Self {
        let cap = min_capacity.max(2).next_power_of_two();
        let buf: Vec<AtomicUsize> = (0..cap).map(|_| AtomicUsize::new(SENTINEL)).collect();
        Self {
            buf: buf.into_boxed_slice(),
            mask: cap - 1,
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    /// Reset to empty. Allocation-free. Caller guarantees no concurrent
    /// push/pop is in flight (between `Graph::run_once` calls this is
    /// trivially true).
    pub(crate) fn reset(&self) {
        for slot in &*self.buf {
            slot.store(SENTINEL, Ordering::Relaxed);
        }
        self.head.store(0, Ordering::Relaxed);
        self.tail.store(0, Ordering::Relaxed);
    }

    /// Push a value. Allocation-free. Returns `Err(())` if the ring is
    /// full — by construction unreachable in `Graph::run_once_borrowed`
    /// because the ring is sized to `n_vertices` and every vertex
    /// becomes ready at most once per dispatch.
    pub(crate) fn push(&self, v: usize) -> Result<(), ()> {
        loop {
            let tail = self.tail.load(Ordering::Acquire);
            let head = self.head.load(Ordering::Acquire);
            if tail.wrapping_sub(head) >= self.buf.len() {
                return Err(());
            }
            match self.tail.compare_exchange(
                tail,
                tail.wrapping_add(1),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    self.buf[tail & self.mask].store(v, Ordering::Release);
                    return Ok(());
                }
                Err(_) => continue,
            }
        }
    }

    /// Pop a value. Allocation-free. Returns `None` if the ring is
    /// empty. SPSC on the consumer side.
    pub(crate) fn pop(&self) -> Option<usize> {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Acquire);
        if head == tail {
            return None;
        }
        let slot = &self.buf[head & self.mask];
        // A producer may have reserved this slot via CAS-tail without
        // yet storing the value; spin briefly until the value lands.
        loop {
            let v = slot.load(Ordering::Acquire);
            if v != SENTINEL {
                slot.store(SENTINEL, Ordering::Release);
                self.head.store(head.wrapping_add(1), Ordering::Release);
                return Some(v);
            }
            std::hint::spin_loop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_pop_in_order() {
        let r = ReadyRing::new(4);
        assert!(r.push(1).is_ok());
        assert!(r.push(2).is_ok());
        assert!(r.push(3).is_ok());
        assert_eq!(r.pop(), Some(1));
        assert_eq!(r.pop(), Some(2));
        assert_eq!(r.pop(), Some(3));
        assert_eq!(r.pop(), None);
    }

    #[test]
    fn capacity_rounds_to_pow2() {
        let r = ReadyRing::new(5);
        // capacity is at least 8 (next pow2 ≥ 5)
        assert!(r.push(1).is_ok());
        for i in 2..=8 {
            assert!(r.push(i).is_ok(), "ring should hold 8 items, failed at {i}");
        }
        // 9th push fills past capacity → Err
        assert!(r.push(99).is_err());
    }

    #[test]
    fn reset_clears_state() {
        let r = ReadyRing::new(4);
        r.push(1).unwrap();
        r.push(2).unwrap();
        r.reset();
        assert_eq!(r.pop(), None);
        r.push(7).unwrap();
        assert_eq!(r.pop(), Some(7));
    }

    #[test]
    fn wraparound_round_trip() {
        let r = ReadyRing::new(4);
        for i in 0..4 {
            r.push(i).unwrap();
            assert_eq!(r.pop(), Some(i));
        }
        // After 4 wraps, head and tail are at 4 each; ring is empty.
        assert_eq!(r.pop(), None);
        // Push four more — should succeed, exercising wraparound.
        for i in 100..104 {
            r.push(i).unwrap();
        }
        for i in 100..104 {
            assert_eq!(r.pop(), Some(i));
        }
    }

    #[test]
    fn mpsc_smoke_two_producers() {
        use std::sync::Arc;
        use std::thread;

        let r = Arc::new(ReadyRing::new(64));
        let r1 = Arc::clone(&r);
        let r2 = Arc::clone(&r);

        let t1 = thread::spawn(move || {
            for i in 0..20 {
                while r1.push(i).is_err() {
                    std::hint::spin_loop();
                }
            }
        });
        let t2 = thread::spawn(move || {
            for i in 100..120 {
                while r2.push(i).is_err() {
                    std::hint::spin_loop();
                }
            }
        });

        let mut seen: Vec<usize> = Vec::with_capacity(40);
        while seen.len() < 40 {
            if let Some(v) = r.pop() {
                seen.push(v);
            } else {
                std::hint::spin_loop();
            }
        }
        t1.join().unwrap();
        t2.join().unwrap();
        seen.sort_unstable();
        let mut expected: Vec<usize> = (0..20).collect();
        expected.extend(100..120);
        assert_eq!(seen, expected);
    }
}
