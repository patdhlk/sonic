//! Forward-compatible declarations for ethercrab integration. Gated
//! on the `bus-integration` cargo feature.
//!
//! This module is intentionally minimal in C5c. It provides:
//!
//! * [`EthercatPduStorage`] — the default [`PduStorage`] type alias
//!   carrying ethercrab's recommended frame pool size.
//! * [`crate::declare_pdu_storage`] — a macro that declares a
//!   `static` of [`EthercatPduStorage`] in application code, ready
//!   to pass into the bring-up flow.
//!
//! ### What's deferred and why
//!
//! The full `bring_up_bus` + `BusHandle::cycle_once` wiring against
//! `ethercrab::MainDevice` was scoped to this commit but pulled back
//! when it became clear that `ethercrab` 0.7's API surface differs in
//! several places from the example code reachable via documentation
//! search. Without an EtherCAT NIC to iterate against, writing 1000+
//! lines of speculative integration code against an API the author
//! cannot exercise produces code that compiles but whose runtime
//! behaviour is unverified — exactly the kind of "trust me" code the
//! framework otherwise avoids.
//!
//! The integration is therefore tracked as a follow-on under
//! :need:`IMPL_0050` and lands when:
//!
//! 1. A developer with `CAP_NET_RAW` on a Linux gateway host can
//!    iterate the bring-up code against a real bus (or the project
//!    moves to a `BusDriver` trait abstraction with a `MockBusDriver`
//!    in dev-dependencies); and
//! 2. The cycle-loop integration with [`crate::CycleScheduler`],
//!    [`crate::wkc::evaluate_wkc`], [`crate::EthercatHealthMonitor`],
//!    and the bridges can be wired without speculation.
//!
//! The pure-logic helpers `sdo`, `scheduler`, `wkc`, `bridge`, and
//! `health` are already in place and exhaustively unit-tested — when
//! the bring-up code lands, those helpers are the load-bearing
//! decision logic and will not need to change.
//!
//! ### Intended bring-up shape (forward reference)
//!
//! The cycle-loop integration is expected to follow this skeleton
//! once the API mismatches are resolved:
//!
//! 1. `let (tx, rx, pdu_loop) = storage.try_split()?` — one
//!    `MainDevice` per storage (`REQ_0312`).
//! 2. `MainDevice::new(pdu_loop, …)` with `dc_static_sync_iterations`
//!    derived from [`crate::EthercatConnectorOptions::distributed_clocks`]
//!    (`REQ_0318`).
//! 3. `tokio::spawn(ethercrab::std::tx_rx_task(interface, tx, rx)?)`
//!    on the [`crate::EthercatGateway`]'s tokio runtime, which opens
//!    an `AF_PACKET` raw socket and runs the TX/RX cycle
//!    (`REQ_0321`, `REQ_0325`).
//! 4. `maindevice.init_single_group::<MAX_SUBDEVICES, PDI_LEN>(…)`
//!    for SubDevice discovery (PRE-OP state).
//! 5. For every [`crate::SubDeviceMap`] in `options.pdo_map()`,
//!    apply the writes from [`crate::pdo_sdo_writes`] via
//!    `subdevice.sdo_write(index, subindex, value)` (`REQ_0314`,
//!    `REQ_0315`).
//! 6. `group.into_op(&maindevice)` — fast-path PRE-OP → OP
//!    (internally PRE-OP → SAFE-OP → OP) (`REQ_0313`).
//! 7. Per-cycle: [`crate::CycleScheduler::poll`] decides fire/skip;
//!    on fire, `group.tx_rx(&maindevice)`; the response's
//!    `working_counter` feeds [`crate::wkc::evaluate_wkc`] which
//!    drives [`crate::EthercatHealthMonitor`] transitions
//!    (`REQ_0317`, `REQ_0319`, `REQ_0320`).
//!
//! `REQ_0312` (single MainDevice), `REQ_0313` (bus reaches OP before
//! traffic), and `REQ_0325` (Linux raw socket) remain `open` in the
//! corpus until that wiring lands.

use ethercrab::PduStorage;

/// PDU pool frame count used by the default storage type
/// ([`EthercatPduStorage`]). Matches ethercrab's recommended size for
/// modest bus topologies.
pub const ETHERCAT_MAX_FRAMES: usize = 16;

/// Maximum single-frame PDU payload size used by the default storage
/// type ([`EthercatPduStorage`]). Per ethercrab convention.
pub const ETHERCAT_MAX_PDU_DATA: usize = 1100;

/// Default [`PduStorage`] type for sonic-connector-ethercat.
///
/// Declare a `static` of this type via
/// [`crate::declare_pdu_storage!`] and pass a reference into the
/// bring-up flow (deferred to the follow-on commit — see the module
/// docs). Each storage can produce one `MainDevice`
/// ([`PduStorage::try_split`] is one-shot), so applications wanting
/// multiple gateways declare one storage per gateway (`REQ_0312`).
pub type EthercatPduStorage =
    PduStorage<ETHERCAT_MAX_FRAMES, { PduStorage::element_size(ETHERCAT_MAX_PDU_DATA) }>;

/// Declare a `static` [`EthercatPduStorage`] with the default frame
/// pool size. Application code calls this once per planned gateway:
///
/// ```ignore
/// sonic_connector_ethercat::declare_pdu_storage!(BUS_STORAGE);
/// ```
///
/// then later passes `&BUS_STORAGE` to the bring-up flow.
#[macro_export]
macro_rules! declare_pdu_storage {
    ($name:ident) => {
        static $name: $crate::bus::EthercatPduStorage = $crate::bus::EthercatPduStorage::new();
    };
}
