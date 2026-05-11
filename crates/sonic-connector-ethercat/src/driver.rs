//! [`BusDriver`] — the trait abstraction every concrete EtherCAT
//! cycle-driving back-end implements. Carved out in C5d so the
//! cycle-loop integration tests don't depend on real EtherCAT
//! hardware.
//!
//! Two known implementations:
//!
//! * [`crate::MockBusDriver`] — synthetic SubDevices, programmable
//!   working-counter sequences, no hardware. Used by the in-tree
//!   integration tests under `tests/runner.rs`.
//! * `EthercrabBusDriver` — wraps `ethercrab::MainDevice`, spawns
//!   `tx_rx_task`, drives the bus through PRE-OP → SAFE-OP → OP
//!   per `REQ_0312` / `REQ_0313` / `REQ_0315`. Tracked as a
//!   follow-on commit ("C5e") — its API requires hardware
//!   iteration to verify, and the trait abstraction defined here is
//!   the integration point that lets `EthercrabBusDriver` land
//!   incrementally without breaking existing tests.
//!
//! Trait methods are async because real bus operations (ethercrab's
//! `tx_rx`, SDO writes) are async-first. The cycle loop lives on the
//! [`crate::EthercatGateway`]'s tokio runtime, so async at the trait
//! boundary keeps the integration natural.

use sonic_connector_core::ConnectorError;

/// Outcome of [`BusDriver::bring_up`]. Carries the per-cycle expected
/// working counter so the cycle loop can compare each
/// `BusDriver::cycle` response against it (`REQ_0319`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BringUp {
    /// Working counter the bus is expected to return on a healthy
    /// cycle. Concrete implementations compute this from the
    /// configured PDO mapping.
    pub expected_wkc: u16,
    /// Number of SubDevices discovered during bring-up. Informational
    /// — used in tracing spans and logs.
    pub subdevice_count: usize,
}

/// Driver-side contract every cycle-loop back-end implements. Two
/// methods: `bring_up` (one-shot at start) and `cycle` (per-tick).
///
/// `Send + 'static` because the cycle loop runs on a tokio task that
/// owns the driver by value.
pub trait BusDriver: Send + 'static {
    /// One-shot initialisation. Discovers SubDevices, applies the
    /// configured PDO mapping via SDO writes, transitions the bus to
    /// OP state. Concrete implementations encode `REQ_0312` /
    /// `REQ_0313` / `REQ_0314` / `REQ_0315` / `REQ_0318` here.
    fn bring_up(
        &mut self,
    ) -> impl core::future::Future<Output = Result<BringUp, ConnectorError>> + Send + '_;

    /// Run one cycle of process-data exchange. Returns the observed
    /// working counter that the cycle loop feeds into
    /// [`crate::wkc::evaluate_wkc`].
    ///
    /// Concrete implementations encode `REQ_0317` (skip-not-catch-up)
    /// only at the *scheduler* level — the driver simply performs
    /// one `tx_rx` per call. Skipping is the cycle loop's
    /// responsibility, never the driver's.
    fn cycle(
        &mut self,
    ) -> impl core::future::Future<Output = Result<u16, ConnectorError>> + Send + '_;
}
