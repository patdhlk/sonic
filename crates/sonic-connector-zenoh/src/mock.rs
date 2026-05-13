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
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
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

    fn subscriber_count(&self) -> usize {
        self.subscribers
            .lock()
            .unwrap()
            .values()
            .map(Vec::len)
            .sum()
    }

    fn queryable_count(&self) -> usize {
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
