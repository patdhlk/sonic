//! [`EthercatRouting`] — typed routing struct identifying one
//! process-data slice on the EtherCAT bus. `REQ_0311`.

use sonic_connector_core::Routing;

/// Direction of the PDO carried in a slice — `RxPdo` flows from
/// MainDevice to SubDevice; `TxPdo` flows back.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdoDirection {
    /// MainDevice → SubDevice (writes from app side).
    Rx,
    /// SubDevice → MainDevice (reads to app side).
    Tx,
}

/// Identifies one process-data slice: the SubDevice's configured
/// address, the PDO direction, the bit offset within the SubDevice's
/// process image, and the bit length of the mapped object.
///
/// Implements [`Routing`] (`REQ_0222`): `Clone + Send + Sync + Debug +
/// 'static`, no methods of its own.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EthercatRouting {
    /// SubDevice configured address on the EtherCAT bus.
    pub subdevice_address: u16,
    /// PDO direction (RxPdo or TxPdo).
    pub direction: PdoDirection,
    /// Bit offset within the SubDevice's process image where the
    /// mapped object begins.
    pub bit_offset: u32,
    /// Bit length of the mapped object.
    pub bit_length: u16,
}

impl EthercatRouting {
    /// Construct routing identifying one process-data slice.
    #[must_use]
    pub const fn new(
        subdevice_address: u16,
        direction: PdoDirection,
        bit_offset: u32,
        bit_length: u16,
    ) -> Self {
        Self {
            subdevice_address,
            direction,
            bit_offset,
            bit_length,
        }
    }
}

impl Routing for EthercatRouting {}
