//! [`ZenohHealthMonitor`] — thin wrapper around `HealthMonitor`.
//!
//! Broadcasts every emitted `HealthEvent` over a `crossbeam_channel`
//! so observers (e.g. `ZenohGateway::subscribe_health` in Z2) can
//! listen.
//!
//! The wrapper centralises two concerns the bare `HealthMonitor` does
//! not own:
//!
//! 1. Thread-safe access. The bare monitor is `&mut`-only; the gateway
//!    side typically holds it behind a `Mutex` because both async tasks
//!    and synchronous observer threads may observe / mutate health.
//! 2. Fan-out. Every successful transition is rebroadcast to one or
//!    more subscribers via a `crossbeam_channel::Sender`.

use std::sync::Mutex;
use std::time::Instant;

use crossbeam_channel::{Receiver, Sender, unbounded};
use sonic_connector_core::{
    ConnectorError, ConnectorHealth, HealthEvent, HealthMonitor, IllegalTransition,
};

use crate::session::SessionState;

/// Health monitor + broadcast channel pair.
#[derive(Debug)]
pub struct ZenohHealthMonitor {
    inner: Mutex<HealthMonitor>,
    tx: Sender<HealthEvent>,
    rx: Receiver<HealthEvent>,
}

impl ZenohHealthMonitor {
    /// Construct a monitor in the initial `Connecting` state with an
    /// unbounded broadcast channel.
    #[must_use]
    pub fn new() -> Self {
        let (tx, rx) = unbounded();
        Self {
            inner: Mutex::new(HealthMonitor::new()),
            tx,
            rx,
        }
    }

    /// Snapshot the current state.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex has been poisoned by a previous
    /// panicked call. The monitor's methods are short and panic-free
    /// in normal operation, so a poisoned lock indicates a serious
    /// bug elsewhere — fail fast rather than mask.
    pub fn current(&self) -> ConnectorHealth {
        self.inner
            .lock()
            .expect("health monitor lock not poisoned")
            .current()
            .clone()
    }

    /// Try to transition to `target`. On success the emitted
    /// `HealthEvent` is broadcast to every subscriber.
    ///
    /// # Errors
    ///
    /// * [`ZenohHealthError::Illegal`] when the from→to pair is not
    ///   allowed per `ARCH_0012`.
    /// * [`ZenohHealthError::BroadcastClosed`] if the broadcast channel
    ///   has lost all subscribers (impossible by construction — `self`
    ///   holds the receive end).
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned by a previous
    /// panicked transition. See [`Self::current`] for the rationale.
    pub fn transition_to(
        &self,
        target: ConnectorHealth,
    ) -> Result<HealthEvent, ZenohHealthError> {
        let event = {
            let mut guard = self.inner.lock().expect("health monitor lock not poisoned");
            guard
                .try_transition_to(target)
                .map_err(ZenohHealthError::Illegal)?
        };
        self.tx
            .send(event.clone())
            .map_err(|_| ZenohHealthError::BroadcastClosed)?;
        Ok(event)
    }

    /// Subscriber-side receiver. Each `Clone` of the returned handle
    /// observes the same stream — `crossbeam_channel` is MPMC.
    #[must_use]
    pub fn subscribe(&self) -> Receiver<HealthEvent> {
        self.rx.clone()
    }

    /// Apply an observed session state to the monitor. Maps the
    /// [`SessionState`] into a [`ConnectorHealth`] target and attempts
    /// the transition. Used by the Z4e health watcher task — each
    /// observed change of the underlying session state pushes one
    /// event onto the broadcast channel via [`Self::transition_to`].
    ///
    /// Mapping (per `ARCH_0012`'s reachable edges):
    ///
    /// * `SessionState::Connecting` → `ConnectorHealth::Connecting`
    /// * `SessionState::Alive`      → `ConnectorHealth::Up`
    /// * `SessionState::Closed`     → `ConnectorHealth::Down`
    ///
    /// Illegal transitions per the health state machine (e.g. observing
    /// `Alive` while already `Up`, or `Connecting` while `Up`) are
    /// dropped silently — the watcher should not panic on a benign
    /// no-op or an unreachable state-machine edge.
    pub(crate) fn apply_state(&self, next: &SessionState) {
        let target = match next {
            SessionState::Connecting => ConnectorHealth::Connecting {
                since: Instant::now(),
            },
            SessionState::Alive => ConnectorHealth::Up,
            SessionState::Closed { reason } => ConnectorHealth::Down {
                reason: reason.clone(),
                since: Instant::now(),
            },
        };
        // Drop illegal-transition errors silently — `Up -> Up` /
        // `Up -> Connecting` etc. are no-ops or unreachable for the
        // watcher's caller. BroadcastClosed is impossible because the
        // monitor holds an internal receiver clone.
        let _ = self.transition_to(target);
    }
}

impl Default for ZenohHealthMonitor {
    fn default() -> Self {
        Self::new()
    }
}

/// Failure modes of [`ZenohHealthMonitor::transition_to`].
#[derive(Debug, thiserror::Error)]
pub enum ZenohHealthError {
    /// Requested from→to transition not allowed by `ARCH_0012`.
    #[error(transparent)]
    Illegal(#[from] IllegalTransition),
    /// Broadcast channel has no receivers — should not happen
    /// because the monitor holds an internal receiver clone.
    #[error("health broadcast channel closed")]
    BroadcastClosed,
}

impl From<ZenohHealthError> for ConnectorError {
    fn from(err: ZenohHealthError) -> Self {
        match err {
            ZenohHealthError::Illegal(e) => Self::stack(e),
            ZenohHealthError::BroadcastClosed => Self::Down {
                reason: "health broadcast closed".to_string(),
            },
        }
    }
}
