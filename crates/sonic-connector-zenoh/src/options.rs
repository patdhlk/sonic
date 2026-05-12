//! Configuration knobs for the Zenoh gateway (`ZenohConnectorOptions`).
//!
//! Covers `REQ_0440` (session mode), `REQ_0443` (locators), `REQ_0425`
//! (query defaults), and `REQ_0404` (bridge capacities). The builder
//! enforces sensible defaults so a `ZenohConnectorOptions::builder().build()`
//! call yields a working peer-mode configuration.

use std::time::Duration;

/// Zenoh session topology. Default: `Peer`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SessionMode {
    /// Peer-to-peer topology (default).
    #[default]
    Peer,
    /// Client topology — connects to a router.
    Client,
    /// Router topology — routes between clients and peers.
    Router,
}

/// A Zenoh locator (e.g. `tcp/127.0.0.1:7447`). Carried verbatim through
/// to `zenoh::Config` in Z4 (`REQ_0443`); the connector does not parse
/// these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locator(String);

impl Locator {
    /// Create a new `Locator` from any string-like value.
    #[must_use]
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// Return the locator string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Default routing target for a query (mirrors Zenoh's `QueryTarget`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryTarget {
    /// Route to the best-matching subscriber.
    BestMatching,
    /// Route to all matching subscribers (default).
    All,
    /// Route to all complete (non-partial) subscribers.
    AllComplete,
}

/// Consolidation strategy for replies to a query (mirrors Zenoh's
/// `ConsolidationMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Consolidation {
    /// Let Zenoh choose automatically.
    Auto,
    /// No consolidation (default).
    None,
    /// Keep only monotonically newer replies.
    Monotonic,
    /// Keep only the latest reply per key.
    Latest,
}

/// Process-wide configuration for the Zenoh gateway.
#[derive(Debug, Clone)]
pub struct ZenohConnectorOptions {
    /// Zenoh session topology.
    pub mode: SessionMode,
    /// Locators to connect to on session open.
    pub connect: Vec<Locator>,
    /// Locators to listen on.
    pub listen: Vec<Locator>,
    /// Default query target.
    pub query_target: QueryTarget,
    /// Default consolidation mode for queries.
    pub query_consolidation: Consolidation,
    /// Default query timeout.
    pub query_timeout: Duration,
    /// Channel capacity for the outbound bridge.
    pub outbound_bridge_capacity: usize,
    /// Channel capacity for the inbound bridge.
    pub inbound_bridge_capacity: usize,
    /// Number of tokio worker threads for the runtime (clamped to at least 1).
    pub tokio_worker_threads: usize,
    /// Minimum number of peers required before the session is considered
    /// ready. `None` means no minimum.
    pub min_peers: Option<usize>,
}

impl ZenohConnectorOptions {
    /// Return a new [`ZenohConnectorOptionsBuilder`] with all defaults set.
    #[must_use]
    pub fn builder() -> ZenohConnectorOptionsBuilder {
        ZenohConnectorOptionsBuilder::default()
    }
}

/// Typed builder for [`ZenohConnectorOptions`].
#[derive(Debug)]
pub struct ZenohConnectorOptionsBuilder {
    mode: SessionMode,
    connect: Vec<Locator>,
    listen: Vec<Locator>,
    query_target: QueryTarget,
    query_consolidation: Consolidation,
    query_timeout: Duration,
    outbound_bridge_capacity: usize,
    inbound_bridge_capacity: usize,
    tokio_worker_threads: usize,
    min_peers: Option<usize>,
}

impl Default for ZenohConnectorOptionsBuilder {
    fn default() -> Self {
        Self {
            mode: SessionMode::Peer,
            connect: Vec::new(),
            listen: Vec::new(),
            query_target: QueryTarget::All,
            query_consolidation: Consolidation::None,
            query_timeout: Duration::from_secs(10),
            outbound_bridge_capacity: 64,
            inbound_bridge_capacity: 64,
            tokio_worker_threads: 1,
            min_peers: None,
        }
    }
}

impl ZenohConnectorOptionsBuilder {
    /// Override the session topology.
    #[must_use]
    pub const fn mode(mut self, m: SessionMode) -> Self {
        self.mode = m;
        self
    }

    /// Append a connect locator (accumulated in insertion order).
    #[must_use]
    pub fn connect(mut self, l: Locator) -> Self {
        self.connect.push(l);
        self
    }

    /// Append a listen locator (accumulated in insertion order).
    #[must_use]
    pub fn listen(mut self, l: Locator) -> Self {
        self.listen.push(l);
        self
    }

    /// Override the default query target.
    #[must_use]
    pub const fn query_target(mut self, t: QueryTarget) -> Self {
        self.query_target = t;
        self
    }

    /// Override the default consolidation mode.
    #[must_use]
    pub const fn query_consolidation(mut self, c: Consolidation) -> Self {
        self.query_consolidation = c;
        self
    }

    /// Override the default query timeout.
    #[must_use]
    pub const fn query_timeout(mut self, d: Duration) -> Self {
        self.query_timeout = d;
        self
    }

    /// Set the outbound bridge channel capacity (clamped to at least 1).
    #[must_use]
    pub fn outbound_bridge_capacity(mut self, n: usize) -> Self {
        self.outbound_bridge_capacity = n.max(1);
        self
    }

    /// Set the inbound bridge channel capacity (clamped to at least 1).
    #[must_use]
    pub fn inbound_bridge_capacity(mut self, n: usize) -> Self {
        self.inbound_bridge_capacity = n.max(1);
        self
    }

    /// Set the number of tokio worker threads (clamped to at least 1).
    #[must_use]
    pub const fn tokio_worker_threads(mut self, n: usize) -> Self {
        self.tokio_worker_threads = if n == 0 { 1 } else { n };
        self
    }

    /// Require at least `n` peers before the session is considered ready.
    #[must_use]
    pub const fn min_peers(mut self, n: usize) -> Self {
        self.min_peers = Some(n);
        self
    }

    /// Consume the builder and return the final [`ZenohConnectorOptions`].
    #[must_use]
    pub fn build(self) -> ZenohConnectorOptions {
        ZenohConnectorOptions {
            mode: self.mode,
            connect: self.connect,
            listen: self.listen,
            query_target: self.query_target,
            query_consolidation: self.query_consolidation,
            query_timeout: self.query_timeout,
            outbound_bridge_capacity: self.outbound_bridge_capacity,
            inbound_bridge_capacity: self.inbound_bridge_capacity,
            tokio_worker_threads: self.tokio_worker_threads,
            min_peers: self.min_peers,
        }
    }
}
