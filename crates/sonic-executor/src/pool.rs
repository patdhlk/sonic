//! Worker pool with per-iteration barrier semantics. See design §9 (M1).

// Pool items are pub(crate) but not yet consumed until Task 7 wires them in.
#![allow(dead_code)]
// redundant_pub_crate fires because the module itself is private; the
// pub(crate) visibility is intentional for when the executor (Task 7) imports
// Pool.
#![allow(clippy::redundant_pub_crate)]

use crate::error::ExecutorError;
use crossbeam_channel::{Receiver, Sender, bounded};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

/// Boxed unit of work submitted into the pool.
type Job = Box<dyn FnOnce() + Send + 'static>;

/// Shared progress tracker — counts jobs submitted vs completed, used for
/// `barrier()`.
#[derive(Default)]
struct Tracker {
    submitted: AtomicUsize,
    completed: AtomicUsize,
    cv: Condvar,
    lock: Mutex<()>,
}

impl Tracker {
    fn submit(&self) {
        self.submitted.fetch_add(1, Ordering::SeqCst);
    }

    fn complete(&self) {
        self.completed.fetch_add(1, Ordering::SeqCst);
        // Acquire+drop the lock to establish happens-before with the waiter,
        // then notify *after* releasing — avoids a wake-then-sleep cycle under
        // high completion rate.
        drop(self.lock.lock().unwrap());
        self.cv.notify_all();
    }

    #[allow(clippy::significant_drop_tightening)]
    fn wait_for_quiescence(&self) {
        // The guard must be held across every cv.wait() call; clippy's
        // suggestion to drop it early would break the condvar contract.
        let mut g = self.lock.lock().unwrap();
        while self.submitted.load(Ordering::SeqCst) != self.completed.load(Ordering::SeqCst) {
            g = self.cv.wait(g).unwrap();
        }
    }
}

/// Worker pool with two modes: `n=0` runs inline; `n>=1` spawns N OS threads.
pub(crate) struct Pool {
    mode: PoolMode,
    tracker: Arc<Tracker>,
}

/// Internal execution mode for the pool.
enum PoolMode {
    /// All jobs run synchronously on the calling thread.
    Inline,
    /// Jobs are dispatched to N worker threads via a bounded channel.
    Threaded {
        /// Sending end of the job channel.
        tx: Sender<Job>,
        /// Worker thread handles, drained on drop.
        handles: Vec<JoinHandle<()>>,
        /// Set to `true` to ask workers to exit after draining.
        shutdown: Arc<std::sync::atomic::AtomicBool>,
    },
}

impl Pool {
    /// Create a new pool. `n_workers == 0` selects inline mode; any positive
    /// value spawns that many OS threads. `attrs` controls thread names,
    /// CPU affinity, and scheduling priority.
    pub(crate) fn new(
        n_workers: usize,
        attrs: crate::thread_attrs::ThreadAttributes,
    ) -> Result<Self, ExecutorError> {
        let tracker = Arc::new(Tracker::default());
        if n_workers == 0 {
            return Ok(Self {
                mode: PoolMode::Inline,
                tracker,
            });
        }

        let (tx, rx): (Sender<Job>, Receiver<Job>) = bounded(n_workers * 4);
        let shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let attrs = Arc::new(attrs);
        let mut handles = Vec::with_capacity(n_workers);
        for i in 0..n_workers {
            let rx = rx.clone();
            let tracker = Arc::clone(&tracker);
            let shutdown = Arc::clone(&shutdown);
            let attrs = Arc::clone(&attrs);
            let name = {
                #[cfg(feature = "thread_attrs")]
                {
                    attrs
                        .name_prefix
                        .as_ref()
                        .map_or_else(|| format!("sonic-pool-{i}"), |p| format!("{p}-{i}"))
                }
                #[cfg(not(feature = "thread_attrs"))]
                {
                    format!("sonic-pool-{i}")
                }
            };
            let h = thread::Builder::new()
                .name(name)
                .spawn(move || {
                    attrs.apply_to_self(i);
                    while !shutdown.load(Ordering::Acquire) {
                        match rx.recv() {
                            Ok(job) => {
                                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
                                tracker.complete();
                            }
                            Err(_) => break,
                        }
                    }
                })
                .map_err(|e| ExecutorError::Builder(format!("spawn worker: {e}")))?;
            handles.push(h);
        }
        Ok(Self {
            mode: PoolMode::Threaded {
                tx,
                handles,
                shutdown,
            },
            tracker,
        })
    }

    /// Submit a job to the pool. In inline mode the job runs immediately on
    /// the calling thread; in threaded mode it is enqueued for a worker.
    #[track_caller]
    pub(crate) fn submit<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        self.tracker.submit();
        match &self.mode {
            PoolMode::Inline => {
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
                self.tracker.complete();
            }
            PoolMode::Threaded { tx, .. } => {
                // Safe to expect: the channel sender lives in self, and self can't be
                // dropped while we hold &self. The only path to a closed channel is
                // Pool::drop, which can't run concurrently with submit().
                tx.send(Box::new(f)).expect("pool channel closed");
            }
        }
    }

    /// Block until every job submitted so far has completed.
    pub(crate) fn barrier(&self) {
        self.tracker.wait_for_quiescence();
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        if let PoolMode::Threaded {
            shutdown,
            handles,
            tx,
        } = &mut self.mode
        {
            shutdown.store(true, Ordering::Release);
            // Replace tx with a fresh closed channel so the original Sender is
            // dropped here. That makes recv on workers return Err(_) and lets
            // the threads exit promptly even if shutdown was checked just
            // before they entered recv().
            let (closed_tx, _) = bounded::<Job>(0);
            let _ = std::mem::replace(tx, closed_tx);
            for h in handles.drain(..) {
                let _ = h.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thread_attrs::ThreadAttributes;
    use std::sync::atomic::AtomicU32;

    #[test]
    fn inline_pool_runs_synchronously() {
        let pool = Pool::new(0, ThreadAttributes::new()).unwrap();
        let counter = Arc::new(AtomicU32::new(0));
        for _ in 0..10 {
            let c = Arc::clone(&counter);
            pool.submit(move || {
                c.fetch_add(1, Ordering::SeqCst);
            });
        }
        pool.barrier();
        assert_eq!(counter.load(Ordering::SeqCst), 10);
    }

    #[test]
    fn threaded_pool_runs_concurrently_and_barriers() {
        let pool = Pool::new(4, ThreadAttributes::new()).unwrap();
        let counter = Arc::new(AtomicU32::new(0));
        for _ in 0..100 {
            let c = Arc::clone(&counter);
            pool.submit(move || {
                std::thread::sleep(std::time::Duration::from_millis(1));
                c.fetch_add(1, Ordering::SeqCst);
            });
        }
        pool.barrier();
        assert_eq!(counter.load(Ordering::SeqCst), 100);
    }

    #[test]
    fn barrier_with_no_work_returns_immediately() {
        let pool = Pool::new(2, ThreadAttributes::new()).unwrap();
        pool.barrier();
        // No assertion — must not deadlock.
    }

    #[test]
    fn submitted_panic_is_caught_and_completion_counted() {
        let pool = Pool::new(2, ThreadAttributes::new()).unwrap();
        pool.submit(|| panic!("kaboom"));
        pool.submit(|| {});
        pool.barrier();
        // Both jobs must be marked complete even though one panicked. If they
        // weren't, barrier would have hung — but we make the postcondition
        // explicit so a future regression of "tracker.complete() skipped on
        // panic" surfaces as an assertion failure rather than a 60s hang.
        assert_eq!(
            pool.tracker.submitted.load(Ordering::SeqCst),
            pool.tracker.completed.load(Ordering::SeqCst),
            "submitted vs completed counters diverged after panic"
        );
    }
}
