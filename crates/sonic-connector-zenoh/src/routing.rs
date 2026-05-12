//! Per-channel routing struct (`ZenohRouting`) and the key-expression /
//! `QoS` types it carries.
//!
//! Implements `REQ_0401`. The routing struct is what plugin code attaches
//! to a `ChannelDescriptor<R: Routing, N>`; invalid key expressions are
//! rejected here, on the plugin side, before any iceoryx2 service is
//! created (so the failure surfaces as `ConnectorError::Configuration`
//! rather than a partial wire-up).

use sonic_connector_core::Routing;

/// Validated Zenoh key expression. Construct via `KeyExprOwned::try_from(&str)`.
///
/// Validation rules (Z1 baseline — additional rules may land in later stages
/// if the real `zenoh::KeyExpr` parser surfaces them):
///
/// * Non-empty.
/// * No leading slash (`/foo` is rejected — Zenoh keys are root-relative).
/// * No trailing slash.
/// * No double slash (each path chunk between slashes must be non-empty).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyExprOwned(String);

impl KeyExprOwned {
    /// Borrow the validated key expression as a `&str`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for KeyExprOwned {
    type Error = KeyExprError;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        validate_key_expr(value)?;
        Ok(Self(value.to_owned()))
    }
}

impl TryFrom<String> for KeyExprOwned {
    type Error = KeyExprError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        validate_key_expr(&value)?;
        Ok(Self(value))
    }
}

/// Errors surfaced from `KeyExprOwned` construction.
#[derive(Debug, thiserror::Error)]
pub enum KeyExprError {
    /// The provided key expression was empty.
    #[error("invalid key_expr: empty string")]
    Empty,
    /// The key expression has a leading `/` (Zenoh keys are root-relative).
    #[error("invalid key_expr: leading '/' is not allowed (Zenoh keys are root-relative)")]
    LeadingSlash,
    /// The key expression has a trailing `/`.
    #[error("invalid key_expr: trailing '/' is not allowed")]
    TrailingSlash,
    /// The key expression has an empty chunk between two slashes.
    #[error("invalid key_expr: empty chunk between '/'")]
    EmptyChunk,
}

fn validate_key_expr(value: &str) -> Result<(), KeyExprError> {
    if value.is_empty() {
        return Err(KeyExprError::Empty);
    }
    if value.starts_with('/') {
        return Err(KeyExprError::LeadingSlash);
    }
    if value.ends_with('/') {
        return Err(KeyExprError::TrailingSlash);
    }
    if value.contains("//") {
        return Err(KeyExprError::EmptyChunk);
    }
    Ok(())
}

/// Congestion-control policy for a Zenoh publisher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CongestionControl {
    /// Block the sender until the message can be buffered.
    Block,
    /// Drop the message rather than block.
    Drop,
}

/// Priority for Zenoh traffic. Higher variants run before lower variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    /// Real-time priority.
    RealTime,
    /// Interactive high priority.
    InteractiveHigh,
    /// Interactive low priority.
    InteractiveLow,
    /// Data high priority.
    DataHigh,
    /// Default data priority.
    Data,
    /// Data low priority.
    DataLow,
    /// Background priority.
    Background,
}

/// Reliability mode for a Zenoh publisher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reliability {
    /// Reliable transport (retransmits dropped packets).
    Reliable,
    /// Best-effort transport (no retransmission).
    BestEffort,
}

/// Per-channel routing for the Zenoh connector. Implements `Routing`.
#[derive(Debug, Clone)]
pub struct ZenohRouting {
    key_expr: KeyExprOwned,
    congestion_control: CongestionControl,
    priority: Priority,
    reliability: Reliability,
    express: bool,
}

impl ZenohRouting {
    /// Create a new routing with the given key expression and the default
    /// `QoS` knobs (`Drop` congestion, `Data` priority, `BestEffort`,
    /// non-express).
    #[must_use]
    pub const fn new(key_expr: KeyExprOwned) -> Self {
        Self {
            key_expr,
            congestion_control: CongestionControl::Drop,
            priority: Priority::Data,
            reliability: Reliability::BestEffort,
            express: false,
        }
    }

    /// Override the congestion-control mode.
    #[must_use]
    pub const fn with_congestion_control(mut self, c: CongestionControl) -> Self {
        self.congestion_control = c;
        self
    }

    /// Override the priority.
    #[must_use]
    pub const fn with_priority(mut self, p: Priority) -> Self {
        self.priority = p;
        self
    }

    /// Override the reliability.
    #[must_use]
    pub const fn with_reliability(mut self, r: Reliability) -> Self {
        self.reliability = r;
        self
    }

    /// Override the express flag (batching opt-out).
    #[must_use]
    pub const fn with_express(mut self, e: bool) -> Self {
        self.express = e;
        self
    }

    /// Borrow the validated key expression.
    #[must_use]
    pub const fn key_expr(&self) -> &KeyExprOwned {
        &self.key_expr
    }

    /// Return the congestion-control policy.
    #[must_use]
    pub const fn congestion_control(&self) -> CongestionControl {
        self.congestion_control
    }

    /// Return the priority.
    #[must_use]
    pub const fn priority(&self) -> Priority {
        self.priority
    }

    /// Return the reliability mode.
    #[must_use]
    pub const fn reliability(&self) -> Reliability {
        self.reliability
    }

    /// Return whether express mode is enabled.
    #[must_use]
    pub const fn express(&self) -> bool {
        self.express
    }
}

impl Routing for ZenohRouting {}
