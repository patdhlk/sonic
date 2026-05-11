//! [`HealthSubscription`] — receive-only handle over a connector's
//! [`HealthEvent`] stream.
//!
//! Deviates from `REQ_0231`'s literal `sonic_executor::Channel<HealthEvent>`
//! return type: that channel kind requires `HealthEvent: ZeroCopySend`,
//! which is incompatible with the rich `String`-bearing [`HealthEvent`]
//! defined in `sonic-connector-core`. We use a `crossbeam_channel`
//! receiver for the in-process case; a follow-on commit introduces a
//! POD wire form (`HealthEventWire`) for cross-process delivery
//! when a real `Connector` needs it.
//!
//! [`HealthEvent`]: sonic_connector_core::HealthEvent

use crossbeam_channel::{Receiver, TryRecvError};
use sonic_connector_core::HealthEvent;

/// Receive-only handle over a connector's [`HealthEvent`] stream.
///
/// Cloneable: each clone observes the same backing channel, so
/// fan-out is supported.
#[derive(Clone, Debug)]
pub struct HealthSubscription {
    rx: Receiver<HealthEvent>,
}

impl HealthSubscription {
    /// Construct from a `crossbeam_channel::Receiver`. Concrete
    /// connectors call this with the receive end of their internal
    /// health-broadcast channel.
    #[must_use]
    pub const fn new(rx: Receiver<HealthEvent>) -> Self {
        Self { rx }
    }

    /// Try to dequeue one event without blocking.
    ///
    /// Returns:
    /// * `Ok(Some(event))` — an event was available.
    /// * `Ok(None)` — no event is currently available.
    ///
    /// # Errors
    ///
    /// Returns an [`HealthSubscriptionError`] when the corresponding
    /// publisher side has been dropped (the connector has been
    /// destroyed). Subsequent calls keep returning the same error.
    pub fn try_next(&self) -> Result<Option<HealthEvent>, HealthSubscriptionError> {
        match self.rx.try_recv() {
            Ok(ev) => Ok(Some(ev)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(HealthSubscriptionError::Disconnected),
        }
    }

    /// Borrow the underlying receiver. Useful for callers that want to
    /// integrate the subscription into a `select!`-style multiplexer.
    #[must_use]
    pub const fn receiver(&self) -> &Receiver<HealthEvent> {
        &self.rx
    }
}

/// Errors surfaced by [`HealthSubscription`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HealthSubscriptionError {
    /// The publisher side of the underlying channel has been dropped.
    /// The connector that produced this subscription is gone.
    #[error("health channel disconnected: connector dropped")]
    Disconnected,
}
