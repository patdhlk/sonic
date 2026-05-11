//! [`EthercatConnectorOptions`] — typed builder configuring an
//! `EthercatConnector` / `EthercatGateway` pair. `ADR_0027`.
//!
//! The PDO mapping is declared as `&'static [SubDeviceMap]` so it
//! lives in `.rodata` and the gateway needs no per-instance heap for
//! it (`REQ_0314`, `REQ_0315`).

use std::time::Duration;

/// One SubDevice's PDO mapping. Application code declares an array of
/// these as a `static` and passes the slice to
/// [`EthercatConnectorOptionsBuilder::pdo_map`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SubDeviceMap {
    /// SubDevice configured address on the EtherCAT bus.
    pub address: u16,
    /// Mapped RxPDO entries (MainDevice → SubDevice flow).
    pub rx_pdos: &'static [PdoEntry],
    /// Mapped TxPDO entries (SubDevice → MainDevice flow).
    pub tx_pdos: &'static [PdoEntry],
}

/// One mapped object within a PDO. `index` is the SDO index of the
/// mapped object; `bit_offset` and `bit_length` position it within the
/// PDO's process data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdoEntry {
    /// SDO index of the mapped object.
    pub index: u16,
    /// Bit offset within the PDO process data.
    pub bit_offset: u16,
    /// Bit length of the mapped object.
    pub bit_length: u16,
}

/// Built `EthercatConnectorOptions`. Constructed via
/// [`EthercatConnectorOptionsBuilder`]; never mutated after build.
#[derive(Clone, Debug)]
pub struct EthercatConnectorOptions {
    cycle_time: Duration,
    distributed_clocks: bool,
    outbound_capacity: usize,
    inbound_capacity: usize,
    network_interface: Option<String>,
    pdo_map: &'static [SubDeviceMap],
    tokio_worker_threads: usize,
}

impl EthercatConnectorOptions {
    /// Start a builder with default values per the spec.
    #[must_use]
    pub const fn builder() -> EthercatConnectorOptionsBuilder {
        EthercatConnectorOptionsBuilder::new()
    }

    /// Cycle duration (`REQ_0316`). Default 2 ms, minimum 1 ms.
    #[must_use]
    pub const fn cycle_time(&self) -> Duration {
        self.cycle_time
    }

    /// Distributed Clocks bring-up flag (`REQ_0318`). Default `false`.
    #[must_use]
    pub const fn distributed_clocks(&self) -> bool {
        self.distributed_clocks
    }

    /// Configured outbound bridge capacity (`REQ_0322`). Default 256.
    #[must_use]
    pub const fn outbound_capacity(&self) -> usize {
        self.outbound_capacity
    }

    /// Configured inbound bridge capacity (`REQ_0322`). Default 256.
    #[must_use]
    pub const fn inbound_capacity(&self) -> usize {
        self.inbound_capacity
    }

    /// Network interface name the gateway will open (e.g. `"eth0"`).
    /// `None` selects the platform default.
    #[must_use]
    pub fn network_interface(&self) -> Option<&str> {
        self.network_interface.as_deref()
    }

    /// PDO mapping descriptor.
    #[must_use]
    pub const fn pdo_map(&self) -> &'static [SubDeviceMap] {
        self.pdo_map
    }

    /// Tokio worker-thread count for the gateway's sidecar
    /// (`ADR_0026`). Default 1.
    #[must_use]
    pub const fn tokio_worker_threads(&self) -> usize {
        self.tokio_worker_threads
    }
}

/// Builder for [`EthercatConnectorOptions`].
#[derive(Clone, Debug)]
pub struct EthercatConnectorOptionsBuilder {
    cycle_time: Duration,
    distributed_clocks: bool,
    outbound_capacity: usize,
    inbound_capacity: usize,
    network_interface: Option<String>,
    pdo_map: &'static [SubDeviceMap],
    tokio_worker_threads: usize,
}

const EMPTY_PDO_MAP: &[SubDeviceMap] = &[];

impl EthercatConnectorOptionsBuilder {
    /// Construct a builder with default values:
    ///
    /// * `cycle_time` — 2 ms (`REQ_0316`).
    /// * `distributed_clocks` — `false` (`REQ_0318`).
    /// * `outbound_capacity` / `inbound_capacity` — 256.
    /// * `network_interface` — `None`.
    /// * `pdo_map` — empty slice; must be set to a non-empty value
    ///   for a useful gateway.
    /// * `tokio_worker_threads` — 1.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            cycle_time: Duration::from_millis(2),
            distributed_clocks: false,
            outbound_capacity: 256,
            inbound_capacity: 256,
            network_interface: None,
            pdo_map: EMPTY_PDO_MAP,
            tokio_worker_threads: 1,
        }
    }

    /// Override the cycle duration. Values below 1 ms are clamped to
    /// 1 ms at [`Self::build`] time (`REQ_0316`).
    #[must_use]
    pub const fn cycle_time(mut self, d: Duration) -> Self {
        self.cycle_time = d;
        self
    }

    /// Enable or disable Distributed Clocks bring-up (`REQ_0318`).
    #[must_use]
    pub const fn distributed_clocks(mut self, on: bool) -> Self {
        self.distributed_clocks = on;
        self
    }

    /// Outbound bridge capacity (`REQ_0322`). Must be positive;
    /// zero is treated as 1 at build time.
    #[must_use]
    pub const fn outbound_capacity(mut self, n: usize) -> Self {
        self.outbound_capacity = n;
        self
    }

    /// Inbound bridge capacity (`REQ_0322`).
    #[must_use]
    pub const fn inbound_capacity(mut self, n: usize) -> Self {
        self.inbound_capacity = n;
        self
    }

    /// Network interface name (e.g. `"eth0"`).
    #[must_use]
    pub fn network_interface(mut self, name: impl Into<String>) -> Self {
        self.network_interface = Some(name.into());
        self
    }

    /// PDO mapping descriptor.
    #[must_use]
    pub const fn pdo_map(mut self, map: &'static [SubDeviceMap]) -> Self {
        self.pdo_map = map;
        self
    }

    /// Tokio worker-thread count (`ADR_0026`). Values below 1 are
    /// clamped to 1 at [`Self::build`] time.
    #[must_use]
    pub const fn tokio_worker_threads(mut self, n: usize) -> Self {
        self.tokio_worker_threads = n;
        self
    }

    /// Finalise. Applies the minimum-cycle-time clamp (1 ms) and
    /// capacity clamps (at least 1).
    #[must_use]
    pub fn build(self) -> EthercatConnectorOptions {
        let cycle_time = self.cycle_time.max(Duration::from_millis(1));
        EthercatConnectorOptions {
            cycle_time,
            distributed_clocks: self.distributed_clocks,
            outbound_capacity: self.outbound_capacity.max(1),
            inbound_capacity: self.inbound_capacity.max(1),
            network_interface: self.network_interface,
            pdo_map: self.pdo_map,
            tokio_worker_threads: self.tokio_worker_threads.max(1),
        }
    }
}

impl Default for EthercatConnectorOptionsBuilder {
    fn default() -> Self {
        Self::new()
    }
}
