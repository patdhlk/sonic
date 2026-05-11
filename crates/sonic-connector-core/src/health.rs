//! Observable health for every connector — [`ConnectorHealth`] enum,
//! [`HealthEvent`] emitted on every transition, and [`HealthMonitor`]
//! that gates transitions against the `ARCH_0012` state machine.
//!
//! State machine (`ARCH_0012`):
//!
//! ```text
//!   [start] → Connecting
//!   Connecting → Up | Down
//!   Up → Degraded | Down | [shutdown]
//!   Degraded → Up | Down
//!   Down → Connecting | [shutdown]
//! ```
//!
//! Same-discriminant transitions (e.g. `Up → Up`) are illegal — they
//! indicate a bug at the call site (the caller failed to debounce or
//! checked the wrong condition).

use std::time::Instant;

/// Uniform health state of every connector — observable via
/// `Connector::health()` and emitted as [`HealthEvent`] on every
/// transition. `REQ_0230`.
#[derive(Clone, Debug)]
pub enum ConnectorHealth {
    /// The connector is fully operational. Underlying stack is connected
    /// and any per-cycle health checks (e.g. `EtherCAT` working counter)
    /// match expectations.
    Up,

    /// The connector is attempting to bring its underlying stack to
    /// operating state. Entered on construction and on every retry from
    /// `Down`.
    Connecting {
        /// When the current connect attempt started (monotonic clock).
        since: Instant,
    },

    /// The stack is connected but some health check failed transiently
    /// — e.g. `EtherCAT` working counter below expected (`REQ_0320`) or
    /// MQTT PUBACK timeout. Recoverable; expect a transition back to
    /// `Up` once the condition clears.
    Degraded {
        /// Human-readable description of the degraded condition.
        reason: String,
    },

    /// The underlying stack reports a hard disconnect or unrecoverable
    /// error. Outbound sends shall return [`crate::ConnectorError::Down`]
    /// (`REQ_0292`).
    Down {
        /// Human-readable description of why the connector went down.
        reason: String,
        /// When the connector entered `Down` (monotonic clock).
        since: Instant,
    },
}

/// Discriminator-only view of [`ConnectorHealth`] — useful in error
/// messages and transition matrices where the variant fields don't
/// matter.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum ConnectorHealthKind {
    /// `Up` discriminator.
    Up,
    /// `Connecting` discriminator.
    Connecting,
    /// `Degraded` discriminator.
    Degraded,
    /// `Down` discriminator.
    Down,
}

impl ConnectorHealth {
    /// Discriminator-only view of `self`.
    #[must_use]
    pub const fn kind(&self) -> ConnectorHealthKind {
        match self {
            Self::Up => ConnectorHealthKind::Up,
            Self::Connecting { .. } => ConnectorHealthKind::Connecting,
            Self::Degraded { .. } => ConnectorHealthKind::Degraded,
            Self::Down { .. } => ConnectorHealthKind::Down,
        }
    }
}

/// Emitted on every transition between [`ConnectorHealth`] variants.
///
/// `REQ_0234`. Consumers (e.g. `sonic-executor` observers via the
/// optional `tracing` feature adapter — `REQ_0273`) receive one event
/// per legal transition.
#[derive(Clone, Debug)]
pub struct HealthEvent {
    /// State the connector left.
    pub from: ConnectorHealth,
    /// State the connector entered.
    pub to: ConnectorHealth,
    /// When the transition occurred (monotonic clock).
    pub at: Instant,
}

/// Illegal-transition error returned by
/// [`HealthMonitor::try_transition_to`] when the requested
/// from→to pair is not allowed by `ARCH_0012`.
#[derive(Copy, Clone, Debug, thiserror::Error)]
#[error("illegal health transition: {from:?} → {to:?} (ARCH_0012)")]
pub struct IllegalTransition {
    /// Discriminator of the current state.
    pub from: ConnectorHealthKind,
    /// Discriminator of the requested target state.
    pub to: ConnectorHealthKind,
}

/// Stateful gate over the `ARCH_0012` state machine.
///
/// Owns the connector's current [`ConnectorHealth`] and emits a
/// [`HealthEvent`] for each legal transition; rejects illegal
/// transitions with [`IllegalTransition`].
#[derive(Debug)]
pub struct HealthMonitor {
    current: ConnectorHealth,
}

impl HealthMonitor {
    /// Construct a monitor in the initial `Connecting` state. The
    /// connector's start-of-life is always `Connecting` per `ARCH_0012`'s
    /// `[*] → Connecting` edge.
    #[must_use]
    pub fn new() -> Self {
        Self {
            current: ConnectorHealth::Connecting {
                since: Instant::now(),
            },
        }
    }

    /// Borrow the current state.
    #[must_use]
    pub const fn current(&self) -> &ConnectorHealth {
        &self.current
    }

    /// Attempt to transition to `target`. On success, returns the
    /// [`HealthEvent`] the caller should publish on the connector's
    /// health channel (`REQ_0231`).
    ///
    /// # Errors
    ///
    /// Returns [`IllegalTransition`] when the from→to pair is not
    /// allowed by `ARCH_0012`. The monitor's internal state is **not**
    /// changed on failure.
    pub fn try_transition_to(
        &mut self,
        target: ConnectorHealth,
    ) -> Result<HealthEvent, IllegalTransition> {
        if !is_legal_transition(self.current.kind(), target.kind()) {
            return Err(IllegalTransition {
                from: self.current.kind(),
                to: target.kind(),
            });
        }
        let to_for_event = target.clone();
        let from = core::mem::replace(&mut self.current, target);
        Ok(HealthEvent {
            from,
            to: to_for_event,
            at: Instant::now(),
        })
    }

    /// Transition or panic. Use in code paths where the transition is
    /// known to be valid by construction (e.g. exhaustive `match` on a
    /// stack-level event). Production gateway code that handles
    /// externally-driven transitions should prefer
    /// [`Self::try_transition_to`].
    ///
    /// # Panics
    ///
    /// Panics if the requested transition is not allowed by
    /// `ARCH_0012`. The panic is intentional — it indicates a bug at the
    /// call site (`TEST_0101`'s "illegal transitions panic in debug
    /// builds").
    pub fn transition_to(&mut self, target: ConnectorHealth) -> HealthEvent {
        self.try_transition_to(target).unwrap_or_else(|e| {
            panic!("HealthMonitor: {e}");
        })
    }
}

impl Default for HealthMonitor {
    fn default() -> Self {
        Self::new()
    }
}

/// Pure transition matrix from `ARCH_0012`. Compares discriminators
/// only — the variants' fields are irrelevant for the legality check.
///
/// The per-edge form below is deliberately verbose so it maps one-to-one
/// onto `ARCH_0012`'s mermaid; flattening it via `Connecting | Degraded`
/// or similar (per clippy's `unnested_or_patterns`) would save a few
/// lines at the cost of obscuring which edges exist.
#[allow(clippy::unnested_or_patterns)]
const fn is_legal_transition(from: ConnectorHealthKind, to: ConnectorHealthKind) -> bool {
    use ConnectorHealthKind::{Connecting, Degraded, Down, Up};
    matches!(
        (from, to),
        (Connecting, Up)
            | (Connecting, Down)
            | (Up, Degraded)
            | (Up, Down)
            | (Degraded, Up)
            | (Degraded, Down)
            | (Down, Connecting)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spot-check: a fresh monitor is in `Connecting`.
    #[test]
    fn new_starts_connecting() {
        let m = HealthMonitor::new();
        assert_eq!(m.current().kind(), ConnectorHealthKind::Connecting);
    }

    /// Spot-check: `is_legal_transition` agrees with the mermaid in
    /// `ARCH_0012` on a representative legal edge.
    #[test]
    fn connecting_to_up_is_legal() {
        assert!(is_legal_transition(
            ConnectorHealthKind::Connecting,
            ConnectorHealthKind::Up
        ));
    }

    /// Spot-check: `is_legal_transition` rejects an obviously illegal
    /// edge (`Up → Connecting` would skip the `Down` step).
    #[test]
    fn up_to_connecting_is_illegal() {
        assert!(!is_legal_transition(
            ConnectorHealthKind::Up,
            ConnectorHealthKind::Connecting
        ));
    }
}
