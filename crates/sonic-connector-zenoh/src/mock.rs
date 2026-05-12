//! In-process [`ZenohSessionLike`] implementation for Layer-1 tests.
//!
//! [`MockZenohSession`] carries a per-key-expression subscriber registry
//! and a settable [`SessionState`]. Publishes are dispatched synchronously
//! to every matching subscriber (key-expression match in Z1 is exact
//! string equality — wildcard matching lands later if a test needs it).
//! Query / queryable operations stub out as `NotImplemented` until Z3.
//!
//! Covers `REQ_0445`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use crate::routing::ZenohRouting;
use crate::session::{
    DoneCallback, PayloadSink, QuerySink, QueryableHandle, SessionError, SessionState,
    SubscriptionHandle, ZenohSessionLike,
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

/// In-process mock session. Pub/sub round-trips through an internal
/// registry; query operations are Z1 stubs.
pub struct MockZenohSession {
    state: RwLock<SessionState>,
    subscribers: SubscriberMap,
    next_sub_id: AtomicU64,
}

impl std::fmt::Debug for MockZenohSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state.read().unwrap().clone();
        f.debug_struct("MockZenohSession")
            .field("state", &state)
            .field("subscriber_count", &self.subscriber_count())
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

impl ZenohSessionLike for MockZenohSession {
    fn state(&self) -> SessionState {
        self.state.read().unwrap().clone()
    }

    fn publish(&self, routing: &ZenohRouting, payload: &[u8]) -> Result<(), SessionError> {
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

    fn subscribe(
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

    fn query(
        &self,
        _routing: &ZenohRouting,
        _payload: &[u8],
        _timeout: Duration,
        _on_reply: PayloadSink,
        _on_done: DoneCallback,
    ) -> Result<(), SessionError> {
        Err(SessionError::NotImplemented("MockZenohSession::query (Z3)"))
    }

    fn declare_queryable(
        &self,
        _routing: &ZenohRouting,
        _on_query: QuerySink,
    ) -> Result<QueryableHandle, SessionError> {
        Err(SessionError::NotImplemented(
            "MockZenohSession::declare_queryable (Z3)",
        ))
    }
}
