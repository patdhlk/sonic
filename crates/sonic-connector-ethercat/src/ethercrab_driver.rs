//! [`EthercrabBusDriver`] — concrete [`BusDriver`] backed by
//! `ethercrab::MainDevice` on Linux. Gated on the `bus-integration`
//! cargo feature.
//!
//! ## Verification posture
//!
//! Compile-checked against `ethercrab` 0.7. Runtime behaviour is
//! **unverified** in this commit — no EtherCAT hardware is available
//! to iterate against. The `tests/ethercrab_driver.rs` test
//! demonstrates the intended bring-up shape under
//! `#[ignore]` + `ETHERCAT_TEST_NIC` so the next contributor with a
//! Linux gateway host (and `CAP_NET_RAW`) can validate the integration
//! end-to-end without rewriting it from scratch.
//!
//! ## Lifetime / ownership model
//!
//! `ethercrab::PduStorage` is `static` by construction (one
//! storage = one `MainDevice`). The driver borrows the storage as
//! `&'static`, performs the one-shot `try_split` inside
//! [`EthercrabBusDriver::bring_up`], and holds the resulting
//! `MainDevice<'static>` + `SubDeviceGroup<...>` + the spawned
//! `tx_rx_task` join handle for the rest of the driver's lifetime.
//! Dropping the driver aborts the join handle so the raw socket is
//! released.
//!
//! ## Working-counter expectation
//!
//! `group.tx_rx` issues an LRW datagram per cycle. A healthy LRW
//! against `N` SubDevices contributes `3 × N` to the working counter
//! (write `+1`, read `+2`). `expected_wkc` is therefore computed as
//! `3 × subdevice_count` after discovery; asymmetric PDO mappings
//! (read-only or write-only SubDevices) will need a per-deployment
//! override — tracked as a follow-on.

use std::time::Duration;

use ethercrab::std::{ethercat_now, tx_rx_task};
use ethercrab::subdevice_group::{Op, PreOp};
use ethercrab::{
    DefaultLock, MainDevice, MainDeviceConfig, SubDeviceGroup, Timeouts, error::Error as EcError,
};
use sonic_connector_core::ConnectorError;

use crate::bus::EthercatPduStorage;
use crate::driver::{BringUp, BusDriver};
use crate::options::EthercatConnectorOptions;
use crate::sdo::{SdoValue, pdo_sdo_writes};

/// Production [`BusDriver`] wrapping `ethercrab::MainDevice`.
///
/// Generic on the bus topology (`MAX_SUBDEVICES`, `MAX_PDI`). Both
/// values are compile-time bounds checked by `ethercrab` during
/// `init_single_group`; oversize topologies fail at bring-up time.
pub struct EthercrabBusDriver<const MAX_SUBDEVICES: usize, const MAX_PDI: usize> {
    storage: &'static EthercatPduStorage,
    interface: String,
    options: EthercatConnectorOptions,
    state: State<MAX_SUBDEVICES, MAX_PDI>,
}

// `Operational` is much larger than `NotInitialised` (carries
// MainDevice + SubDeviceGroup + JoinHandle); box it so the enum
// discriminant footprint stays small.
enum State<const MAX_SUBDEVICES: usize, const MAX_PDI: usize> {
    /// Pre-bring-up. The storage has not yet been split; no
    /// `MainDevice` exists.
    NotInitialised,
    /// Post-bring-up. `tx_rx_task` is spawned and the group is in OP.
    Operational(Box<OperationalState<MAX_SUBDEVICES, MAX_PDI>>),
}

struct OperationalState<const MAX_SUBDEVICES: usize, const MAX_PDI: usize> {
    /// MainDevice owns the `PduLoop` it was constructed with.
    maindevice: MainDevice<'static>,
    /// OP-state group. `cycle` calls `group.tx_rx(&maindevice)`.
    group: SubDeviceGroup<MAX_SUBDEVICES, MAX_PDI, DefaultLock, Op>,
    /// JoinHandle for the `tx_rx_task`. Aborted on `Drop`.
    tx_rx_task: tokio::task::JoinHandle<()>,
}

impl<const MAX_SUBDEVICES: usize, const MAX_PDI: usize>
    EthercrabBusDriver<MAX_SUBDEVICES, MAX_PDI>
{
    /// Construct a driver bound to `storage`. `bring_up` must be
    /// called (via the [`BusDriver`] trait) before any `cycle`
    /// invocation.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectorError::InvalidDescriptor`] if
    /// `options.network_interface()` is `None` — the gateway needs an
    /// interface name to open the raw socket on (`REQ_0325`).
    pub fn new(
        storage: &'static EthercatPduStorage,
        options: EthercatConnectorOptions,
    ) -> Result<Self, ConnectorError> {
        let interface = options
            .network_interface()
            .ok_or_else(|| {
                ConnectorError::InvalidDescriptor(
                    "EthercatConnectorOptions::network_interface is required for \
                     EthercrabBusDriver"
                        .into(),
                )
            })?
            .to_owned();
        Ok(Self {
            storage,
            interface,
            options,
            state: State::NotInitialised,
        })
    }
}

impl<const MAX_SUBDEVICES: usize, const MAX_PDI: usize> Drop
    for EthercrabBusDriver<MAX_SUBDEVICES, MAX_PDI>
{
    fn drop(&mut self) {
        if let State::Operational(op) = std::mem::replace(&mut self.state, State::NotInitialised) {
            op.tx_rx_task.abort();
        }
    }
}

impl<const MAX_SUBDEVICES: usize, const MAX_PDI: usize> BusDriver
    for EthercrabBusDriver<MAX_SUBDEVICES, MAX_PDI>
{
    async fn bring_up(&mut self) -> Result<BringUp, ConnectorError> {
        if matches!(self.state, State::Operational(_)) {
            return Err(ConnectorError::InvalidDescriptor(
                "EthercrabBusDriver::bring_up called twice".into(),
            ));
        }

        // One-shot storage split — REQ_0312 ("single MainDevice per
        // gateway"). PduStorage's `try_split` enforces this at the
        // ethercrab layer; we surface it as our error type.
        let (pdu_tx, pdu_rx, pdu_loop) = self.storage.try_split().map_err(|()| {
            ConnectorError::InvalidDescriptor(
                "EthercatPduStorage already split — declare a fresh storage per gateway".into(),
            )
        })?;

        let dc_iters = if self.options.distributed_clocks() {
            // ethercrab's recommended drift-compensation iterations for
            // DC bring-up. The actual `tx_rx_dc` cycle path is a
            // follow-on; this just enables MainDevice-internal
            // sync-time alignment.
            10_000
        } else {
            0
        };

        let maindevice = MainDevice::new(
            pdu_loop,
            Timeouts {
                wait_loop_delay: Duration::from_millis(2),
                mailbox_response: Duration::from_secs(1),
                ..Timeouts::default()
            },
            MainDeviceConfig {
                dc_static_sync_iterations: dc_iters,
                ..MainDeviceConfig::default()
            },
        );

        // Spawn the TX/RX loop on the current tokio runtime. The task
        // owns the raw socket; aborting it (in `Drop`) releases it.
        let tx_rx_future = tx_rx_task(&self.interface, pdu_tx, pdu_rx)
            .map_err(|e| ConnectorError::stack(IoError(format!("tx_rx_task: {e}"))))?;
        let tx_rx_task_handle = tokio::spawn(async move {
            // The future itself may return an `Err` if the socket
            // dies; for a no-op tokio task we ignore the result —
            // surfacing it requires another channel. The cycle loop
            // will already error on `tx_rx` if the bus dies.
            let _ = tx_rx_future.await;
        });

        // PRE-OP discovery (REQ_0313 — bus must reach OP before
        // traffic; we walk PRE-OP → SAFE-OP → OP via the fast path).
        let group: SubDeviceGroup<MAX_SUBDEVICES, MAX_PDI, DefaultLock, PreOp> = maindevice
            .init_single_group::<MAX_SUBDEVICES, MAX_PDI>(ethercat_now)
            .await
            .map_err(map_ec_error)?;

        // Apply the static PDO mapping (REQ_0315). Each SubDeviceMap's
        // SDO sequence is generated by the pure-logic helper
        // [`crate::sdo::pdo_sdo_writes`] (exhaustively unit-tested in
        // C5b); here we just execute it against the discovered
        // SubDevices.
        for map in self.options.pdo_map() {
            apply_pdo_mapping_for_subdevice(&maindevice, &group, *map).await?;
        }

        // PRE-OP → SAFE-OP → OP via the fast path (REQ_0313).
        let group = group.into_op(&maindevice).await.map_err(map_ec_error)?;

        let subdevice_count = group.len();
        // Expected WKC per cycle: LRW datagram contributes +3 per
        // SubDevice (write +1, read +2). Asymmetric mappings need a
        // per-deployment override — tracked as a follow-on. `u16::MAX`
        // saturates if a topology somehow exceeds 21845 SubDevices.
        let expected_wkc = u16::try_from(subdevice_count)
            .unwrap_or(u16::MAX)
            .saturating_mul(3);

        self.state = State::Operational(Box::new(OperationalState {
            maindevice,
            group,
            tx_rx_task: tx_rx_task_handle,
        }));

        Ok(BringUp {
            expected_wkc,
            subdevice_count,
        })
    }

    async fn cycle(&mut self) -> Result<u16, ConnectorError> {
        let State::Operational(op) = &self.state else {
            return Err(ConnectorError::Down {
                reason: "EthercrabBusDriver::cycle called before bring_up".into(),
            });
        };
        let response = op.group.tx_rx(&op.maindevice).await.map_err(map_ec_error)?;
        Ok(response.working_counter)
    }
}

/// Apply the SDO write sequence for one `SubDeviceMap`. Locates the
/// matching SubDevice by `configured_address` and dispatches each
/// `SdoValue` to ethercrab's typed `sdo_write`.
async fn apply_pdo_mapping_for_subdevice<const MAX_SUBDEVICES: usize, const MAX_PDI: usize>(
    maindevice: &MainDevice<'_>,
    group: &SubDeviceGroup<MAX_SUBDEVICES, MAX_PDI, DefaultLock, PreOp>,
    map: crate::options::SubDeviceMap,
) -> Result<(), ConnectorError> {
    let writes = pdo_sdo_writes(&map);
    if writes.is_empty() {
        return Ok(());
    }

    let mut found = false;
    for subdevice in group.iter(maindevice) {
        if subdevice.configured_address() != map.address {
            continue;
        }
        found = true;
        for write in &writes {
            match write.value {
                SdoValue::U8(v) => subdevice
                    .sdo_write(write.index, write.subindex, v)
                    .await
                    .map_err(map_ec_error)?,
                SdoValue::U16(v) => subdevice
                    .sdo_write(write.index, write.subindex, v)
                    .await
                    .map_err(map_ec_error)?,
            }
        }
        break;
    }

    if !found {
        return Err(ConnectorError::InvalidDescriptor(format!(
            "SubDevice {:#06x} declared in pdo_map but not present on the bus",
            map.address
        )));
    }
    Ok(())
}

fn map_ec_error(e: EcError) -> ConnectorError {
    ConnectorError::stack(EcWrappedError(format!("{e:?}")))
}

#[derive(Debug)]
struct EcWrappedError(String);

impl core::fmt::Display for EcWrappedError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "ethercrab: {}", self.0)
    }
}

impl std::error::Error for EcWrappedError {}

#[derive(Debug)]
struct IoError(String);

impl core::fmt::Display for IoError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "io: {}", self.0)
    }
}

impl std::error::Error for IoError {}
