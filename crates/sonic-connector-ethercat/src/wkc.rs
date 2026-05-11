//! Working-counter-based health policy. `REQ_0319`, `REQ_0320`.
//!
//! Per cycle the gateway computes an expected WKC from the PDO
//! mapping and reads back the observed WKC from
//! `ethercrab::SubDeviceGroup::tx_rx`. This module provides the
//! decision: did the working counter match (→ `Up`) or come up short
//! (→ `Degraded` with a reason naming the offending cycle)?

/// Pure decision: did the working counter match expectation?
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WkcVerdict {
    /// `observed >= expected` — bus is healthy on this cycle.
    Match,
    /// `observed < expected` — at least one SubDevice didn't respond
    /// per the configured mapping. The fields are forwarded into the
    /// `ConnectorHealth::Degraded::reason` message.
    Mismatch {
        /// Working counter the SubDeviceGroup actually returned.
        observed: u16,
        /// Working counter the gateway expected from its mapping.
        expected: u16,
    },
}

impl WkcVerdict {
    /// Compose the human-readable reason string surfaced via
    /// [`sonic_connector_core::ConnectorHealth::Degraded`]. Only
    /// meaningful when `self` is [`WkcVerdict::Mismatch`].
    #[must_use]
    pub fn degraded_reason(&self, cycle_index: u64) -> Option<String> {
        match *self {
            Self::Match => None,
            Self::Mismatch { observed, expected } => Some(format!(
                "working counter below expected: cycle {cycle_index}, observed {observed}, expected {expected}"
            )),
        }
    }
}

/// Compare an observed WKC against the expected value. `observed >=
/// expected` is a [`WkcVerdict::Match`]; anything less is
/// [`WkcVerdict::Mismatch`] (`REQ_0319` / `REQ_0320`).
#[must_use]
pub const fn evaluate_wkc(expected: u16, observed: u16) -> WkcVerdict {
    if observed >= expected {
        WkcVerdict::Match
    } else {
        WkcVerdict::Mismatch { observed, expected }
    }
}
