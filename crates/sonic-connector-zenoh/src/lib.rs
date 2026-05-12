//! Zenoh reference connector for the sonic-connector framework.
//!
//! Implements `BB_0040` and the Zenoh-specific surface declared in
//! `IMPL_0060`. This crate is structured in stages — Stage Z1 lands
//! the protocol-agnostic core (routing, options, bridges, session
//! trait, mock session, health monitor). Later stages add the
//! `Connector` trait impl (Z2), query handles (Z3), and the real
//! `zenoh::Session` wrapper (Z4).
//!
//! See `spec/architecture/connector.rst` for the full `IMPL_0060`
//! directive and `docs/superpowers/specs/2026-05-12-zenoh-connector-design.md`
//! for the design context.

pub mod bridge;
pub mod health;
pub mod mock;
pub mod options;
pub mod routing;
pub mod session;

pub use bridge::{InboundBridge, InboundOutcome, OutboundBridge, OutboundError};
// pub use health::ZenohHealthMonitor;
// pub use mock::MockZenohSession;
pub use options::{
    Consolidation, Locator, QueryTarget, SessionMode, ZenohConnectorOptions,
    ZenohConnectorOptionsBuilder,
};
pub use routing::{
    CongestionControl, KeyExprOwned, Priority, Reliability, ZenohRouting,
};
// pub use session::{FrameKind, ReplyFrame, SessionState, ZenohSessionLike};
