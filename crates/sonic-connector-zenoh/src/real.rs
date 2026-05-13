//! `RealZenohSession` — `ZenohSessionLike` over `zenoh::Session` (`REQ_0444`).
//!
//! Thin wrapper around the real zenoh-1.x session so `ZenohConnector`
//! can monomorphise over the live protocol stack the same way it does
//! over `MockZenohSession`. Compiled only with the `zenoh-integration`
//! cargo feature.
//!
//! # API notes
//!
//! The zenoh-1.x `Config` type does NOT expose typed setters
//! (`set_mode`, `connect.endpoints.push`, ...). The only public mutator
//! is [`zenoh::Config::insert_json5`], which takes a dotted key path
//! and a JSON5-encoded value. We use that to translate
//! [`ZenohConnectorOptions`] fields onto a default config.
//!
//! `Publisher`/`Subscriber`/`Queryable` carry a lifetime parameter
//! tied to the key-expression input. Passing an owned `String` makes
//! the resulting handle `'static`, which is what we need to stash in
//! `Arc`s / `Box<dyn Any + Send + Sync>` opaque handles.
//!
//! The queryable callback runs on a zenoh-internal task; to send a
//! reply from inside that callback we use the [`zenoh::Wait`] trait
//! (sync resolver), which is the documented pattern for callback
//! contexts.

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex, RwLock};
use std::time::Duration;

use tokio::sync::Mutex as AsyncMutex;
use zenoh::Wait;

use crate::options::{SessionMode, ZenohConnectorOptions};
use crate::routing::ZenohRouting;
use crate::session::{
    DoneCallback, PayloadSink, QueryReplier, QuerySink, QueryableHandle, SessionError,
    SessionState, SubscriptionHandle, ZenohSessionLike,
};

/// Shared, sync-safe form of the `QuerySink` user callback. We hold
/// it in an `Arc` so the per-query closure dispatched from zenoh's
/// callback thread can invoke it without ownership transfer.
type SharedQuerySink = Arc<dyn Fn(&[u8], QueryReplier) + Send + Sync + 'static>;

/// A live `zenoh::Session` plus a publisher cache, implementing
/// [`ZenohSessionLike`] over the real zenoh-1.x stack (`REQ_0444`).
pub struct RealZenohSession {
    inner: Arc<zenoh::Session>,
    publishers: AsyncMutex<HashMap<String, Arc<zenoh::pubsub::Publisher<'static>>>>,
    state: RwLock<SessionState>,
}

impl std::fmt::Debug for RealZenohSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RealZenohSession").finish_non_exhaustive()
    }
}

impl RealZenohSession {
    /// Open a new real zenoh session, translating
    /// [`ZenohConnectorOptions`] (`REQ_0440`, `REQ_0443`) into a
    /// [`zenoh::Config`] and awaiting [`zenoh::open`].
    ///
    /// # Errors
    /// Returns [`SessionError::OpenFailed`] if config construction or
    /// the underlying `zenoh::open` call fails.
    pub async fn open(opts: &ZenohConnectorOptions) -> Result<Self, SessionError> {
        let config = build_zenoh_config(opts)?;
        let session = zenoh::open(config)
            .await
            .map_err(|e| SessionError::OpenFailed {
                reason: format!("zenoh::open: {e}"),
            })?;
        Ok(Self {
            inner: Arc::new(session),
            publishers: AsyncMutex::new(HashMap::new()),
            state: RwLock::new(SessionState::Alive),
        })
    }
}

/// Build a [`zenoh::Config`] from [`ZenohConnectorOptions`] via
/// [`zenoh::Config::insert_json5`] (the only public mutator on the
/// zenoh-1.x `Config` surface).
fn build_zenoh_config(opts: &ZenohConnectorOptions) -> Result<zenoh::Config, SessionError> {
    let mut config = zenoh::Config::default();

    let mode_str = match opts.mode {
        SessionMode::Peer => "\"peer\"",
        SessionMode::Client => "\"client\"",
        SessionMode::Router => "\"router\"",
    };
    config
        .insert_json5("mode", mode_str)
        .map_err(|e| SessionError::OpenFailed {
            reason: format!("set mode: {e}"),
        })?;

    if !opts.connect.is_empty() {
        let endpoints_json = endpoints_to_json5(&opts.connect);
        config
            .insert_json5("connect/endpoints", &endpoints_json)
            .map_err(|e| SessionError::OpenFailed {
                reason: format!("set connect/endpoints: {e}"),
            })?;
    }

    if !opts.listen.is_empty() {
        let endpoints_json = endpoints_to_json5(&opts.listen);
        config
            .insert_json5("listen/endpoints", &endpoints_json)
            .map_err(|e| SessionError::OpenFailed {
                reason: format!("set listen/endpoints: {e}"),
            })?;
    }

    Ok(config)
}

/// Serialise a slice of [`crate::options::Locator`]s into a JSON5
/// array literal suitable for `Config::insert_json5`. We hand-roll the
/// serialiser to keep this module free of an extra json dep and to
/// keep the escaping rules explicit — locators are ASCII with no
/// embedded quotes in normal use, but we still escape defensively.
fn endpoints_to_json5(locators: &[crate::options::Locator]) -> String {
    let mut out = String::from("[");
    for (i, loc) in locators.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('"');
        for ch in loc.as_str().chars() {
            match ch {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                _ => out.push(ch),
            }
        }
        out.push('"');
    }
    out.push(']');
    out
}

impl ZenohSessionLike for RealZenohSession {
    fn state(&self) -> SessionState {
        self.state
            .read()
            .expect("session state lock not poisoned")
            .clone()
    }

    async fn publish(
        &self,
        routing: &ZenohRouting,
        payload: &[u8],
    ) -> Result<(), SessionError> {
        let key = routing.key_expr().as_str().to_owned();
        let publisher = {
            let mut map = self.publishers.lock().await;
            if let Some(p) = map.get(&key) {
                Arc::clone(p)
            } else {
                let p = self
                    .inner
                    .declare_publisher(key.clone())
                    .await
                    .map_err(|e| SessionError::DeclarationFailed {
                        reason: format!("declare_publisher '{key}': {e}"),
                    })?;
                let arc = Arc::new(p);
                map.insert(key.clone(), Arc::clone(&arc));
                arc
            }
        };
        publisher
            .put(payload.to_vec())
            .await
            .map_err(|e| SessionError::PublishFailed {
                reason: format!("put: {e}"),
            })?;
        Ok(())
    }

    async fn subscribe(
        &self,
        routing: &ZenohRouting,
        sink: PayloadSink,
    ) -> Result<SubscriptionHandle, SessionError> {
        let key = routing.key_expr().as_str().to_owned();
        let subscriber = self
            .inner
            .declare_subscriber(key.clone())
            .callback(move |sample: zenoh::sample::Sample| {
                let bytes = sample.payload().to_bytes();
                sink(&bytes);
            })
            .await
            .map_err(|e| SessionError::DeclarationFailed {
                reason: format!("declare_subscriber '{key}': {e}"),
            })?;
        Ok(SubscriptionHandle(Box::new(subscriber)))
    }

    async fn query(
        &self,
        routing: &ZenohRouting,
        payload: &[u8],
        timeout: Duration,
        on_reply: PayloadSink,
        on_done: DoneCallback,
    ) -> Result<(), SessionError> {
        let key = routing.key_expr().as_str().to_owned();

        self.inner
            .get(key.clone())
            .payload(payload.to_vec())
            .timeout(timeout)
            .callback(move |reply: zenoh::query::Reply| {
                if let Ok(sample) = reply.result() {
                    let bytes = sample.payload().to_bytes();
                    on_reply(&bytes);
                }
                // Reply-side errors are silently dropped here; Z4h
                // introduces tracing for peer-side error reporting.
            })
            .await
            .map_err(|e| SessionError::PublishFailed {
                reason: format!("get: {e}"),
            })?;

        // Fire on_done after the get future has been dispatched. The
        // gateway's outer timeout is authoritative for stream end —
        // this on_done signals the querier task that the call
        // completed normally.
        on_done();
        Ok(())
    }

    async fn declare_queryable(
        &self,
        routing: &ZenohRouting,
        on_query: QuerySink,
    ) -> Result<QueryableHandle, SessionError> {
        let key = routing.key_expr().as_str().to_owned();
        // QuerySink is `Box<dyn Fn(...) + Send + Sync + 'static>` per
        // session.rs; convert to Arc so the inner Fn can be cloned
        // into each per-query callback invocation.
        let on_query_arc: SharedQuerySink = Arc::from(on_query);

        let queryable = self
            .inner
            .declare_queryable(key.clone())
            .callback(move |query: zenoh::query::Query| {
                let payload_vec: Vec<u8> = query
                    .payload()
                    .map(|p| p.to_bytes().into_owned())
                    .unwrap_or_default();

                // Share the Query across the two PayloadSink/DoneCallback
                // closures we hand to the user; both will sync-reply via
                // `zenoh::Wait::wait()`. Mutex is sync (StdMutex) — these
                // closures run on zenoh's callback thread, not in an
                // async context.
                let query_cell = Arc::new(StdMutex::new(Some(query)));
                let query_for_reply = Arc::clone(&query_cell);
                let query_for_terminate = Arc::clone(&query_cell);

                let replier = QueryReplier {
                    reply: Box::new(move |body: &[u8]| {
                        let guard = query_for_reply
                            .lock()
                            .expect("query lock not poisoned");
                        if let Some(q) = guard.as_ref() {
                            let reply_builder = q.reply(q.key_expr().clone(), body.to_vec());
                            let _ = reply_builder.wait();
                        }
                    }),
                    terminate: Box::new(move || {
                        // Drop the query; zenoh finalises the reply
                        // stream when the Query handle is dropped.
                        let _ = query_for_terminate
                            .lock()
                            .expect("query lock not poisoned")
                            .take();
                    }),
                };

                (on_query_arc)(&payload_vec, replier);
            })
            .await
            .map_err(|e| SessionError::DeclarationFailed {
                reason: format!("declare_queryable '{key}': {e}"),
            })?;

        Ok(QueryableHandle(Box::new(queryable)))
    }
}
