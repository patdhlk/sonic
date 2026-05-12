//! Bounded bridges between the sonic-executor side (plugin) and the
//! tokio sidecar (gateway).
//!
//! * [`OutboundBridge`] — plugin → gateway. Saturation surfaces as
//!   [`OutboundError::BackPressure`] (`REQ_0404`, `REQ_0405`).
//! * [`InboundBridge`] — gateway → plugin. Saturation surfaces as
//!   [`InboundOutcome::Dropped`] carrying the running dropped-count
//!   so the gateway can emit `HealthEvent::DroppedInbound { count }`
//!   (`REQ_0404`, `REQ_0406`).
//!
//! The shape of the two types is intentionally identical to
//! `sonic_connector_ethercat::bridge` — `REQ_0404`/`0405`/`0406` are
//! the direct analogs of `REQ_0322`/`0323`/`0324`, and a unified shape
//! keeps the framework-level health-emission logic uniform.

use std::sync::atomic::{AtomicU64, Ordering};

use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};

/// Plugin → gateway bridge. Bounded capacity is fixed at construction.
#[derive(Debug)]
pub struct OutboundBridge<T> {
    tx: Sender<T>,
    rx: Receiver<T>,
    capacity: usize,
}

/// Errors surfaced from [`OutboundBridge::try_send`].
#[derive(Debug, thiserror::Error)]
pub enum OutboundError<T> {
    /// Channel is full — the message was rejected; the caller can
    /// retry or surface back-pressure to the application. `REQ_0405`.
    #[error("outbound bridge full (capacity exceeded)")]
    BackPressure(T),
    /// All receivers have been dropped — the gateway has been
    /// destroyed.
    #[error("outbound bridge disconnected")]
    Disconnected(T),
}

impl<T> OutboundError<T> {
    /// Recover the message that failed to send.
    pub fn into_inner(self) -> T {
        match self {
            Self::BackPressure(t) | Self::Disconnected(t) => t,
        }
    }
}

impl<T> OutboundBridge<T> {
    /// Construct a bridge with the given bounded capacity. The
    /// caller-supplied value is the channel's bound — every
    /// `try_send` past that many in-flight messages returns
    /// [`OutboundError::BackPressure`].
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.max(1);
        let (tx, rx) = bounded(cap);
        Self {
            tx,
            rx,
            capacity: cap,
        }
    }

    /// Try to send a message without blocking.
    pub fn try_send(&self, msg: T) -> Result<(), OutboundError<T>> {
        match self.tx.try_send(msg) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(t)) => Err(OutboundError::BackPressure(t)),
            Err(TrySendError::Disconnected(t)) => Err(OutboundError::Disconnected(t)),
        }
    }

    /// Receive the next message, blocking until one is available or
    /// the channel disconnects. Used on the gateway side.
    pub fn recv(&self) -> Option<T> {
        self.rx.recv().ok()
    }

    /// Try to receive without blocking.
    pub fn try_recv(&self) -> Option<T> {
        self.rx.try_recv().ok()
    }

    /// Channel's bounded capacity.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }
}

/// Gateway → plugin bridge. On overflow the message is dropped and a
/// running dropped-count is incremented (`REQ_0406`).
#[derive(Debug)]
pub struct InboundBridge<T> {
    tx: Sender<T>,
    rx: Receiver<T>,
    capacity: usize,
    dropped: AtomicU64,
}

/// Outcome of [`InboundBridge::try_send`].
#[derive(Debug)]
pub enum InboundOutcome {
    /// The message was enqueued.
    Sent,
    /// The channel was full — the message was dropped, and the
    /// caller is given the running drop-count (`REQ_0406`). The
    /// gateway should emit `HealthEvent::DroppedInbound { count }`
    /// based on this value.
    Dropped {
        /// Cumulative count of inbound messages dropped on this
        /// bridge since construction.
        count: u64,
    },
}

impl<T> InboundBridge<T> {
    /// Construct a bridge with bounded capacity.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.max(1);
        let (tx, rx) = bounded(cap);
        Self {
            tx,
            rx,
            capacity: cap,
            dropped: AtomicU64::new(0),
        }
    }

    /// Try to enqueue an inbound message. On full, the message is
    /// dropped and the dropped-count is incremented (`REQ_0406`).
    pub fn try_send(&self, msg: T) -> InboundOutcome {
        match self.tx.try_send(msg) {
            Ok(()) => InboundOutcome::Sent,
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                let count = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
                InboundOutcome::Dropped { count }
            }
        }
    }

    /// Try to receive without blocking.
    pub fn try_recv(&self) -> Option<T> {
        self.rx.try_recv().ok()
    }

    /// Channel's bounded capacity.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Cumulative count of inbound drops since construction.
    #[must_use]
    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}
