Requirements
============

System-level requirements. The spec is organised under three peer
top-level features:

* :need:`FEAT_0010` "PLC runtime heart on iceoryx2" — sonic-executor
  framed as the runtime heart of a soft-real-time PLC. See
  :doc:`plc-runtime`.
* :need:`FEAT_0030` "Connector framework" — the general-purpose framework
  for bridging sonic-executor applications to external protocols. See
  :doc:`connector`.
* :need:`FEAT_0040` "Bounded global allocator" — workspace
  infrastructure providing a static, pre-allocated, fixed-block
  ``#[global_allocator]`` for sonic binaries that require
  compile-time guarantees on memory usage. See :doc:`bounded-alloc`.
* :need:`FEAT_0050` "Device-driver codegen toolchain" — build-time
  layered crates that translate EtherCAT ESI XML into strongly-typed
  Rust device drivers, consumed by ``sonic-connector-ethercat`` and
  any other ethercrab user. See :doc:`device-codegen`.
* :need:`FEAT_0060` "CANopen device-driver codegen toolchain" —
  build-time layered crates that translate CANopen EDS (CiA 306)
  files into strongly-typed Rust device drivers, with a shared
  ``fieldbus-od-core`` OD IR co-owned by the EtherCAT toolchain.
  See :doc:`canopen-codegen`.

Each ``req`` directive ``:satisfies:`` one ``feat`` parent; each
capability-cluster ``feat`` ``:satisfies:`` its top-level umbrella feature.

.. toctree::
   :maxdepth: 2

   plc-runtime
   connector
   bounded-alloc
   device-codegen
   canopen-codegen

Requirements at a glance
------------------------

.. needtable::
   :types: req
   :columns: id, title, status, satisfies
   :show_filters:
