//! In-process [`ZenohSessionLike`] implementation for Layer-1 tests.
//!
//! [`MockZenohSession`] carries a per-key-expression subscriber registry
//! and a parallel queryable registry, plus a settable [`SessionState`].
//! Publishes are dispatched synchronously to every matching subscriber
//! (key-expression match here is exact string equality — wildcard
//! matching lands later if a test needs it). Queries are dispatched
//! synchronously to every matching queryable; the mock does NOT enforce
//! the `timeout` parameter (that is the gateway's responsibility per
//! `REQ_0425`).
//!
//! Covers `REQ_0445`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use crate::routing::ZenohRouting;
use crate::session::{
    DoneCallback, PayloadSink, QueryReplier, QuerySink, QueryableHandle, SessionError,
    SessionState, SubscriptionHandle, ZenohSessionLike,
};

/// Shared-ownership sink stored per subscriber entry. Using `Arc` (rather
/// than `Box`) lets `publish` clone the `Arc`s under the lock and then
/// invoke the callbacks without holding the lock.
type SharedSink = Arc<dyn Fn(&[u8]) + Send + Sync + 'static>;

struct SubscriberEntry {
    id: u64,
    sink: SharedSink,
}

type SubscriberMap = Arc<Mutex<HashMap<String, Vec<SubscriberEntry>>>>;

/// Shared-ownership sink stored per queryable entry. Mirrors `SharedSink`
/// for the queryable registry side.
type QueryableSink = Arc<dyn Fn(&[u8], QueryReplier) + Send + Sync + 'static>;

struct QueryableEntry {
    id: u64,
    sink: QueryableSink,
}

type QueryableMap = Arc<Mutex<HashMap<String, Vec<QueryableEntry>>>>;

/// Shared cell holding the caller's `on_done` callback until the last
/// queryable to terminate fires it. `FnOnce` can't be cloned, so the
/// `Mutex<Option<...>>` cell lets exactly one terminate path take and
/// invoke the callback.
type DoneCell = Arc<Mutex<Option<DoneCallback>>>;

/// In-process mock session. Pub/sub and query round-trip through internal
/// registries; `timeout` is not enforced (gateway-layer concern).
pub struct MockZenohSession {
    state: RwLock<SessionState>,
    subscribers: SubscriberMap,
    next_sub_id: AtomicU64,
    queryables: QueryableMap,
    next_qable_id: AtomicU64,
    /// Test-only knob (`set_query_hangs`). When `true`, `query`
    /// short-circuits to `Ok(())` without invoking any subscriber /
    /// `on_done` callback — used to exercise the gateway's
    /// `tokio::time::timeout` enforcement path (`TEST_0307`).
    query_hangs: AtomicBool,
    /// Test-only: callbacks captured by `query(...)` when
    /// `query_hangs` was true at the time of the call. Keyed by the
    /// routing's key-expression string. Used by `force_late_reply`
    /// to exercise the gateway's late-reply dedup path (`Z5c`).
    ///
    /// If two `query()` calls hit the same key before
    /// `force_late_reply`, the second silently overwrites the first.
    hung_callbacks: Mutex<HashMap<String, (PayloadSink, DoneCallback)>>,
    /// Reported peer count for the health-watcher polling path
    /// (`ZenohSessionLike::peer_count`). Defaults to `usize::MAX`
    /// so tests that don't configure `min_peers` see `Up` rather
    /// than accidentally tripping into `Degraded`.
    peer_count: AtomicUsize,
}

impl std::fmt::Debug for MockZenohSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state.read().unwrap().clone();
        f.debug_struct("MockZenohSession")
            .field("state", &state)
            .field("subscriber_count", &self.subscriber_count())
            .field("queryable_count", &self.queryable_count())
            .finish_non_exhaustive()
    }
}

impl Default for MockZenohSession {
    fn default() -> Self {
        Self::new()
    }
}

impl MockZenohSession {
    /// Create a fresh mock session starting in [`SessionState::Alive`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: RwLock::new(SessionState::Alive),
            subscribers: Arc::new(Mutex::new(HashMap::new())),
            next_sub_id: AtomicU64::new(1),
            queryables: Arc::new(Mutex::new(HashMap::new())),
            next_qable_id: AtomicU64::new(1),
            query_hangs: AtomicBool::new(false),
            hung_callbacks: Mutex::new(HashMap::new()),
            peer_count: AtomicUsize::new(usize::MAX),
        }
    }

    /// Force the session into the given state. Tests use this to walk
    /// the health state machine.
    ///
    /// # Panics
    ///
    /// Panics if the internal state lock is poisoned (only possible if
    /// a previous thread panicked while holding the write lock).
    pub fn set_state(&self, state: SessionState) {
        *self.state.write().unwrap() = state;
    }

    /// Test-only knob: when `true`, `query` succeeds at the entry point
    /// but never invokes its callbacks. Used to exercise the gateway's
    /// `tokio::time::timeout` enforcement path against
    /// `MockZenohSession` (`TEST_0307`).
    ///
    /// Intentionally NOT `cfg(test)`-gated — the mock ships always per
    /// `REQ_0445`; feature-gating this method would force feature-flag
    /// complexity onto downstream test crates.
    pub fn set_query_hangs(&self, hang: bool) {
        self.query_hangs.store(hang, Ordering::Release);
    }

    /// Test-only: simulate a late reply arriving on the
    /// most-recently-captured callbacks for `key`. Used together with
    /// `set_query_hangs` to exercise the gateway's late-reply dedup
    /// path (`Z5c`).
    ///
    /// `payload` is passed verbatim to `on_reply`; `on_done` is fired
    /// immediately after, mirroring the normal end-of-stream sequence
    /// on the mock's happy path.
    ///
    /// Returns `true` if a captured callback pair was found and fired,
    /// `false` if no `query_hangs`-captured pair exists for that key.
    ///
    /// # Panics
    /// Panics only if the `hung_callbacks` mutex is poisoned, which
    /// would require another thread to panic while holding the lock.
    pub fn force_late_reply(&self, key: &str, payload: &[u8]) -> bool {
        let entry = self
            .hung_callbacks
            .lock()
            .expect("hung_callbacks poisoned")
            .remove(key);
        if let Some((on_reply, on_done)) = entry {
            on_reply(payload);
            on_done();
            true
        } else {
            false
        }
    }

    /// Test-only knob: override the reported peer count. Default is
    /// `usize::MAX` (no constraint). Set a finite value to drive the
    /// health watcher into / out of `Degraded` against a configured
    /// `min_peers` floor.
    pub fn set_peer_count(&self, n: usize) {
        self.peer_count.store(n, Ordering::Release);
    }

    /// Sum of subscriber callbacks across all key-expression buckets.
    /// Used by Z4d's lifecycle test to confirm `SubscriptionHandle`'s
    /// `Drop` impl removes the callback when the connector drops.
    ///
    /// # Panics
    /// Panics only if the subscribers mutex is poisoned, which would
    /// require another thread to panic while holding the lock.
    pub fn subscriber_count(&self) -> usize {
        self.subscribers
            .lock()
            .unwrap()
            .values()
            .map(Vec::len)
            .sum()
    }

    /// Sum of queryable callbacks across all key-expression buckets.
    /// Counterpart to [`Self::subscriber_count`] for queryables.
    ///
    /// # Panics
    /// Panics only if the queryables mutex is poisoned, which would
    /// require another thread to panic while holding the lock.
    pub fn queryable_count(&self) -> usize {
        self.queryables
            .lock()
            .unwrap()
            .values()
            .map(Vec::len)
            .sum()
    }
}

/// Drop guard returned inside [`SubscriptionHandle`]. Removing it from
/// the registry on drop tears down the subscription.
struct SubscriptionGuard {
    id: u64,
    key: String,
    subscribers: SubscriberMap,
}

impl Drop for SubscriptionGuard {
    fn drop(&mut self) {
        if let Ok(mut map) = self.subscribers.lock() {
            if let Some(entries) = map.get_mut(&self.key) {
                entries.retain(|e| e.id != self.id);
                if entries.is_empty() {
                    map.remove(&self.key);
                }
            }
        }
    }
}

/// Drop guard returned inside [`QueryableHandle`]. Removing it from the
/// registry on drop tears down the queryable.
struct QueryableGuard {
    id: u64,
    key: String,
    queryables: QueryableMap,
}

impl Drop for QueryableGuard {
    fn drop(&mut self) {
        if let Ok(mut map) = self.queryables.lock() {
            if let Some(entries) = map.get_mut(&self.key) {
                entries.retain(|e| e.id != self.id);
                if entries.is_empty() {
                    map.remove(&self.key);
                }
            }
        }
    }
}

impl ZenohSessionLike for MockZenohSession {
    fn state(&self) -> SessionState {
        self.state.read().unwrap().clone()
    }

    fn peer_count(&self) -> usize {
        self.peer_count.load(Ordering::Acquire)
    }

    async fn publish(
        &self,
        routing: &ZenohRouting,
        payload: &[u8],
    ) -> Result<(), SessionError> {
        if !matches!(*self.state.read().unwrap(), SessionState::Alive) {
            return Err(SessionError::NotAlive {
                reason: "mock session not alive".into(),
            });
        }
        let key = routing.key_expr().as_str().to_owned();
        // Clone the Arc<dyn Fn> handles under the lock, then release the
        // lock before invoking them.  This avoids holding a MutexGuard
        // across the callbacks (which could cause lock ordering issues or
        // trigger clippy::significant_drop_tightening).
        let sinks: Vec<SharedSink> = self
            .subscribers
            .lock()
            .unwrap()
            .get(&key)
            .map(|entries| entries.iter().map(|e| e.sink.clone()).collect())
            .unwrap_or_default();
        for sink in sinks {
            sink(payload);
        }
        Ok(())
    }

    async fn subscribe(
        &self,
        routing: &ZenohRouting,
        sink: PayloadSink,
    ) -> Result<SubscriptionHandle, SessionError> {
        let key = routing.key_expr().as_str().to_owned();
        let id = self.next_sub_id.fetch_add(1, Ordering::Relaxed);
        // Convert the caller's Box<dyn Fn> into an Arc<dyn Fn> so the sink
        // can be cheaply cloned during publish dispatch.
        let shared: SharedSink = Arc::from(sink);
        let entry = SubscriberEntry {
            id,
            sink: shared,
        };
        self.subscribers
            .lock()
            .unwrap()
            .entry(key.clone())
            .or_default()
            .push(entry);

        let guard = SubscriptionGuard {
            id,
            key,
            subscribers: self.subscribers.clone(),
        };
        Ok(SubscriptionHandle(Box::new(guard)))
    }

    async fn query(
        &self,
        routing: &ZenohRouting,
        payload: &[u8],
        _timeout: Duration,
        on_reply: PayloadSink,
        on_done: DoneCallback,
    ) -> Result<(), SessionError> {
        if !matches!(*self.state.read().unwrap(), SessionState::Alive) {
            return Err(SessionError::NotAlive {
                reason: "mock session not alive".into(),
            });
        }

        // Test-only: never resolve the future so the gateway's
        // `tokio::time::timeout` wrapper actually has something to
        // time out on (`TEST_0307`). A bare `return Ok(())` would let
        // the timeout see a resolved future and never fire.
        //
        // Capture the callbacks so `force_late_reply` can fire them
        // later (`Z5c` — exercises the gateway's late-reply dedup
        // path). We store by key-expression string — sufficient for
        // the single-query-per-key test pattern.
        if self.query_hangs.load(Ordering::Acquire) {
            let _ = payload;
            self.hung_callbacks
                .lock()
                .expect("hung_callbacks poisoned")
                .insert(
                    routing.key_expr().as_str().to_owned(),
                    (on_reply, on_done),
                );
            std::future::pending::<()>().await;
            unreachable!("std::future::pending() never resolves");
        }

        // Snapshot the matching queryables under the lock, then release
        // the lock before invoking any user-supplied callback.
        let key = routing.key_expr().as_str().to_owned();
        let queryables: Vec<QueryableSink> = self
            .queryables
            .lock()
            .unwrap()
            .get(&key)
            .map(|entries| entries.iter().map(|e| Arc::clone(&e.sink)).collect())
            .unwrap_or_default();

        // No queryables matched — fire `on_done` immediately. We do NOT
        // invoke `on_reply` in this path.
        if queryables.is_empty() {
            (on_done)();
            return Ok(());
        }

        // Multiple queryables share the same caller-supplied callbacks.
        // `on_reply` is wrapped in an `Arc<dyn Fn>` so each replier can
        // hold its own clone; `on_done` is `FnOnce`, so it sits in a
        // shared cell and the LAST queryable to terminate fires it.
        let on_reply_arc: SharedSink = Arc::from(on_reply);
        let on_done_cell: DoneCell = Arc::new(Mutex::new(Some(on_done)));
        let pending = Arc::new(AtomicUsize::new(queryables.len()));

        for sink in &queryables {
            let on_reply_clone = Arc::clone(&on_reply_arc);
            let done_cell_clone = Arc::clone(&on_done_cell);
            let pending_clone = Arc::clone(&pending);

            let replier = QueryReplier {
                reply: Box::new(move |body: &[u8]| {
                    (on_reply_clone)(body);
                }),
                terminate: Box::new(move || {
                    // `fetch_sub` returns the previous value; when the
                    // previous value was 1, this terminate is the last
                    // one and should fire `on_done`.
                    let remaining = pending_clone.fetch_sub(1, Ordering::AcqRel);
                    if remaining == 1 {
                        // Hoist the `take()` so the temporary MutexGuard
                        // drops before we invoke the callback.
                        let cb = done_cell_clone.lock().unwrap().take();
                        if let Some(cb) = cb {
                            (cb)();
                        }
                    }
                }),
            };
            sink(payload, replier);
        }

        Ok(())
    }

    async fn declare_queryable(
        &self,
        routing: &ZenohRouting,
        on_query: QuerySink,
    ) -> Result<QueryableHandle, SessionError> {
        let key = routing.key_expr().as_str().to_owned();
        let id = self.next_qable_id.fetch_add(1, Ordering::Relaxed);
        // Convert the caller's Box<dyn Fn> into an Arc<dyn Fn> so the
        // sink can be cheaply cloned during query dispatch.
        let shared: QueryableSink = Arc::from(on_query);
        let entry = QueryableEntry { id, sink: shared };
        self.queryables
            .lock()
            .unwrap()
            .entry(key.clone())
            .or_default()
            .push(entry);

        let guard = QueryableGuard {
            id,
            key,
            queryables: Arc::clone(&self.queryables),
        };
        Ok(QueryableHandle(Box::new(guard)))
    }
}
