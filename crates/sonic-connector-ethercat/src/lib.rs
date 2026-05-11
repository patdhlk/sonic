//! EtherCAT reference connector — `BB_0030` / `FEAT_0041`.
//!
//! This crate is being delivered in stages. The current commit (C5a)
//! lands the protocol-agnostic core:
//!
//! * [`routing::EthercatRouting`] (`REQ_0311`).
//! * [`options::EthercatConnectorOptions`] typed builder + the
//!   `&'static [SubDeviceMap]` PDO descriptor shape (`REQ_0314`,
//!   `REQ_0316`, `REQ_0322`, `ADR_0027`).
//! * [`bridge::OutboundBridge`] / [`bridge::InboundBridge`] with
//!   `BackPressure` and `DroppedInbound` semantics (`REQ_0322`–
//!   `REQ_0324`).
//! * [`health::EthercatHealthMonitor`] wrapping
//!   `sonic_connector_core::HealthMonitor` and broadcasting
//!   [`HealthEvent`]s through a `crossbeam_channel`.
//! * [`gateway::EthercatGateway`] — owns a tokio runtime that is
//!   joined on `Drop` with a 5-second budget (`ADR_0026`,
//!   `REQ_0321`).
//! * [`connector::EthercatConnector`] — implements
//!   [`sonic_connector_host::Connector`] (`REQ_0310`).
//!
//! `ethercrab` integration (MainDevice, SDO writes to `0x1C12` /
//! `0x1C13`, Distributed Clocks bring-up, `tx_rx_task`) lands in a
//! follow-on commit. Until then the gateway side does not actually
//! drive a bus — the trait surface is in place, the bridge / health
//! / lifecycle plumbing is exercised by unit tests, and the typed
//! routing / options types match `REQ_0311` / `REQ_0314`.
//!
//! [`HealthEvent`]: sonic_connector_core::HealthEvent

#![warn(missing_docs)]
// Allow EtherCAT domain identifiers (SubDevice, MainDevice, RxPdo /
// TxPdo, CAP_NET_RAW, etc.) to appear in docstrings without backticks.
// The framework's other crates accept this lint, but EtherCAT
// terminology repeats too often inside our own doc comments to be
// worth backticking individually.
#![allow(clippy::doc_markdown)]

pub mod bridge;
#[cfg(feature = "bus-integration")]
pub mod bus;
pub mod connector;
pub mod driver;
#[cfg(feature = "bus-integration")]
pub mod ethercrab_driver;
pub mod gateway;
pub mod health;
pub mod mock;
pub mod options;
pub mod routing;
pub mod runner;
pub mod scheduler;
pub mod sdo;
pub mod wkc;

pub use bridge::{InboundBridge, InboundOutcome, OutboundBridge, OutboundError};
pub use connector::EthercatConnector;
pub use driver::{BringUp, BusDriver};
#[cfg(feature = "bus-integration")]
pub use ethercrab_driver::EthercrabBusDriver;
pub use gateway::EthercatGateway;
pub use health::EthercatHealthMonitor;
pub use mock::MockBusDriver;
pub use options::{
    EthercatConnectorOptions, EthercatConnectorOptionsBuilder, PdoEntry, SubDeviceMap,
};
pub use routing::{EthercatRouting, PdoDirection};
pub use runner::{CycleReport, CycleRunner};
pub use scheduler::{CycleDecision, CycleScheduler};
pub use sdo::{SM_ASSIGN_RX_PDO, SM_ASSIGN_TX_PDO, SdoValue, SdoWrite, pdo_sdo_writes};
pub use wkc::{WkcVerdict, evaluate_wkc};
