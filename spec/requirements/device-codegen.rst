Device-driver codegen
=====================

This page captures the requirements for the **device-driver codegen
toolchain**: a layered set of crates that translates EtherCAT ESI XML
device descriptions into strongly-typed Rust driver modules at build
time, with zero runtime XML parsing and no dependency on the
``sonic-connector-ethercat`` runtime.

The decomposition mirrors the convention established in
:doc:`connector` and :doc:`plc-runtime`:

* **Top-level umbrella feature** — :need:`FEAT_0050` — peer to
  :need:`FEAT_0010` (PLC runtime heart), :need:`FEAT_0030` (Connector
  framework), and :need:`FEAT_0040` (Bounded global allocator). The
  codegen toolchain is a build-time concern orthogonal to the runtime
  connector framework; it is not bound to sonic-executor or
  sonic-connector and could be consumed by any ethercrab user.
* **Capability-cluster sub-features** — one per crate-layer concern,
  each ``:satisfies:`` :need:`FEAT_0050`.
* **Requirements** — concrete shall-clauses that ``:satisfies:`` a
  capability-cluster feature.

This round covers EtherCAT only (ESI XML → typed driver structs).
CANopen / EDS support is explicitly out of scope; the architecture
preserves the option to extract a shared object-dictionary IR later
(see :need:`ADR_0073`).

Top-level umbrella
------------------

.. feat:: Device-driver codegen toolchain
   :id: FEAT_0050
   :status: open

   A layered set of Rust crates that consumes EtherCAT Slave
   Information (ESI) XML files and emits strongly-typed driver
   modules at build time. The toolchain is organised as four layers
   that depend only leftwards:

   1. **Parse layer** — ``ethercat-esi``: XML → typed IR, ``no_std``
      + ``alloc``. No knowledge of codegen or ethercrab.
   2. **Codegen layer** — ``ethercat-esi-codegen`` (IR →
      ``TokenStream`` via a ``CodegenBackend`` trait) plus
      ``ethercat-esi-codegen-ethercrab`` (the one concrete backend
      shipped in this round).
   3. **Tooling layer** — ``ethercat-esi-build`` (build.rs glue),
      ``ethercat-esi-cli`` (``cargo esi expand`` / ``cargo esi list``
      one-shot tools), and ``ethercat-esi-verify`` (diff ESI XML
      against captured SII EEPROM ``.bin`` dumps).
   4. **Runtime trait crate** — ``ethercat-esi-rt``: the
      ``EsiDevice`` / ``EsiConfigurable`` traits the generated
      drivers implement.

   The ``sonic-connector-ethercat`` crate (see :need:`FEAT_0041`) is
   not part of this toolchain. It sits one layer above as a thin
   adapter that maps any ``EsiDevice`` into the
   ``ethercat_hal::EthercatDevice`` trait it already consumes. No
   change to :need:`FEAT_0041`'s runtime contracts is required by
   this spec.

----

Capability clusters
-------------------

The umbrella decomposes into seven capability clusters. Each cluster
is a sub-feature ``:satisfies:`` :need:`FEAT_0050`, with concrete
shall-clauses underneath.

ESI parser
~~~~~~~~~~

.. feat:: ESI parser
   :id: FEAT_0051
   :status: open
   :satisfies: FEAT_0050

   A pure parser crate. Reads ESI XML, emits a typed in-memory IR.
   Knows nothing about codegen, ethercrab, or sonic-executor. Suitable
   for any downstream tool — codegen, network configurator, simulator,
   verifier.

.. req:: Pure parse function with no I/O
   :id: REQ_0500
   :status: open
   :satisfies: FEAT_0051

   The crate shall expose ``parse(xml: &str) -> Result<EsiFile,
   EsiError>``. The function shall perform no filesystem or network
   I/O; the caller is responsible for reading the XML bytes.

.. req:: no_std + alloc compatible
   :id: REQ_0501
   :status: open
   :satisfies: FEAT_0051

   The crate shall be ``#![no_std]`` with an ``alloc`` dependency so
   it can run inside ``proc-macro``, ``build.rs``, embedded build
   tooling, or a hosted CLI without pulling in a default-features
   ``std`` surface.

.. req:: quick-xml + serde backend
   :id: REQ_0502
   :status: open
   :satisfies: FEAT_0051

   The crate shall implement parsing on top of ``quick-xml`` with
   ``serde`` deserialisation. Hand-written ``Read``-based parsing is
   rejected — schema maintenance lives in the ``serde`` derives.

.. req:: Parser does not depend on ethercrab or codegen
   :id: REQ_0503
   :status: open
   :satisfies: FEAT_0051

   The ``ethercat-esi`` crate shall not declare ``ethercrab`` or
   any codegen crate as a dependency. A downstream tool that only
   needs the IR shall not be forced to compile the codegen layer.

.. req:: IR carries identity, PDO maps, mailbox, DC, and OD
   :id: REQ_0504
   :status: open
   :satisfies: FEAT_0051

   The IR shall represent, per device: ``Identity`` (vendor id,
   product code, revision), ``Vec<SyncManager>``, ``Vec<Pdo>`` for
   TxPDOs and RxPDOs, ``Option<Mailbox>`` capturing CoE/EoE/FoE
   support and InitCmds, ``Option<DistributedClock>``, and a
   ``Vec<DictEntry>`` for the object dictionary. The OD field shall
   be present in the IR unconditionally so non-codegen consumers can
   inspect it; codegen-side emission of OD tables is feature-gated
   per :need:`REQ_0533`.

.. req:: Vendor-specific extensions captured as opaque blobs
   :id: REQ_0505
   :status: open
   :satisfies: FEAT_0051

   Vendor-specific ESI extensions (e.g. Beckhoff ``<Vendor:...>``
   elements) shall be retained in the IR as opaque ``RawXml`` blobs
   carrying the element name, attributes, and inner text/children.
   The parser shall not hard-fail on unknown vendor elements;
   downstream tools may inspect or ignore them.

.. req:: Parse errors carry line and column
   :id: REQ_0506
   :status: open
   :satisfies: FEAT_0051

   ``EsiError`` variants raised during parsing shall carry the
   source line and column of the offending construct so build-time
   diagnostics point at the failing ESI file location.

IR and codegen backend trait
~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. feat:: IR and codegen backend trait
   :id: FEAT_0052
   :status: open
   :satisfies: FEAT_0050

   The codegen-side IR (an extension of the parser IR with naming /
   collision policy applied) and the ``CodegenBackend`` trait that
   lets multiple emitters share that IR. This crate
   (``ethercat-esi-codegen``) knows nothing about XML and nothing
   about ethercrab.

.. req:: CodegenBackend trait shape
   :id: REQ_0510
   :status: open
   :satisfies: FEAT_0052

   The crate shall define a ``CodegenBackend`` trait with
   ``fn emit_device(&self, device: &esi::Device) -> Result<TokenStream,
   CodegenError>`` and ``fn emit_module_root(&self, devices: &[esi::Device])
   -> Result<TokenStream, CodegenError>``. The top-level entry point
   shall be ``fn generate<B: CodegenBackend>(esi: &EsiFile, backend:
   &B) -> Result<TokenStream, CodegenError>``.

.. req:: Naming policy is owned by codegen, not the backend
   :id: REQ_0511
   :status: open
   :satisfies: FEAT_0052

   The ``ethercat-esi-codegen`` crate shall sanitise ESI product
   names into valid Rust identifiers (e.g. ``EL3001-0000`` →
   ``EL3001_0000``) before invoking the backend. Backends shall
   receive idents pre-validated; they shall not be expected to
   re-implement sanitisation.

.. req:: Revision collision handled deterministically
   :id: REQ_0512
   :status: open
   :satisfies: FEAT_0052

   When two devices in the input set share a product name but differ
   in revision (e.g. ``EL3204`` rev ``0x00100000`` vs rev
   ``0x00110000``), the codegen layer shall disambiguate them using
   a deterministic suffix derived from the revision, producing
   distinct Rust idents (``EL3204_REV0010`` vs ``EL3204_REV0011``).
   Ordering of input files shall not affect the generated idents.

.. req:: Common PDO entry types deduplicated
   :id: REQ_0513
   :status: open
   :satisfies: FEAT_0052

   When two or more devices' PDOs include structurally identical
   entry layouts (same field order, same bit lengths, same data
   types), the codegen layer shall emit one shared PDO entry struct
   referenced by both devices rather than two duplicated structs.
   Structural equality is the deduplication key — names do not need
   to match.

.. req:: Emission target is proc_macro2 TokenStream
   :id: REQ_0514
   :status: open
   :satisfies: FEAT_0052

   The codegen layer shall produce ``proc_macro2::TokenStream``
   values and assemble them with ``quote!``. String-templated
   emission (``format!`` + write) is rejected — token-level
   construction preserves span / hygiene and yields rustfmt-able
   output via ``prettyplease``.

ethercrab backend
~~~~~~~~~~~~~~~~~

.. feat:: ethercrab codegen backend
   :id: FEAT_0053
   :status: open
   :satisfies: FEAT_0050

   The opinionated, concrete backend that emits per-device structs
   implementing the runtime traits in :need:`FEAT_0054`. This is the
   only crate in the toolchain that depends on ``ethercrab``.

.. req:: Backend crate is the sole ethercrab dependency
   :id: REQ_0520
   :status: open
   :satisfies: FEAT_0053

   ``ethercat-esi-codegen-ethercrab`` shall be the only crate in the
   toolchain that declares ``ethercrab`` (any version) as a
   dependency. Neither ``ethercat-esi``, ``ethercat-esi-codegen``,
   ``ethercat-esi-build``, nor ``ethercat-esi-verify`` shall depend
   on ``ethercrab``.

.. req:: One device struct per ESI device entry
   :id: REQ_0521
   :status: open
   :satisfies: FEAT_0053

   For each ``<Device>`` element parsed from the input ESI files,
   the backend shall emit exactly one Rust struct named per the
   sanitised product ident (per :need:`REQ_0511` and
   :need:`REQ_0512`), deriving ``Debug + Default + Clone``.

.. req:: SubDeviceIdentity const emitted per device
   :id: REQ_0522
   :status: open
   :satisfies: FEAT_0053

   For each generated device struct, the backend shall emit an
   accompanying ``pub const <IDENT>_REV<REV>: SubDeviceIdentity =
   SubDeviceIdentity { vendor_id, product_id, revision };`` so
   identity-driven dispatch (per :need:`REQ_0525`) can use a static
   table.

.. req:: PDO assignment alternatives emitted as sum type
   :id: REQ_0523
   :status: open
   :satisfies: FEAT_0053

   When an ESI device declares multiple PDO assignment alternatives
   (typically "Standard" / "Compact"), the backend shall emit a
   ``<IDENT>PdoAssignment`` enum with one variant per alternative.
   Modelling alternatives with ``Option<…>`` fields on the device
   struct is rejected — every alternative is a closed, named choice.

.. req:: One PDO struct per assignment alternative
   :id: REQ_0524
   :status: open
   :satisfies: FEAT_0053

   For each variant of ``<IDENT>PdoAssignment``, the backend shall
   emit a corresponding ``<IDENT>Pdo<Variant>`` struct that holds
   the typed PDO entries for that variant. The device struct's
   ``pdo`` field shall be a sum type whose variants embed these
   per-alternative structs.

.. req:: Generated module root exposes a registry
   :id: REQ_0525
   :status: open
   :satisfies: FEAT_0053

   The module root emitted by ``emit_module_root`` shall expose a
   ``registry!()`` declarative macro (or equivalent generated
   ``static`` table) that maps each emitted device's
   ``SubDeviceIdentity`` to a factory closure returning
   ``Box<dyn EsiDevice>``. Identity-based dispatch in downstream
   code (e.g. ``sonic-connector-ethercat``) shall be reducible to a
   ``HashMap`` lookup against this table.

.. req:: Generated code compiles under no_std + alloc
   :id: REQ_0526
   :status: open
   :satisfies: FEAT_0053

   The emitted device modules shall compile under ``#![no_std]`` +
   ``alloc`` so generated drivers are usable from embedded contexts.
   The backend shall not emit ``std::``-qualified paths in
   generated code.

Runtime trait surface
~~~~~~~~~~~~~~~~~~~~~

.. feat:: Runtime trait surface
   :id: FEAT_0054
   :status: open
   :satisfies: FEAT_0050

   The minimal trait pair the generated devices implement and the
   ``sonic-connector-ethercat`` adapter consumes. Lives in a tiny
   ``ethercat-esi-rt`` crate so the runtime contract is not coupled
   to either the codegen or the connector.

.. req:: EsiDevice trait shape
   :id: REQ_0530
   :status: open
   :satisfies: FEAT_0054

   The crate shall define an ``EsiDevice`` trait with
   ``const IDENTITY: SubDeviceIdentity``, ``fn input_len(&self) ->
   usize``, ``fn output_len(&self) -> usize``,
   ``fn decode_inputs(&mut self, bits: &BitSlice<u8, Lsb0>) ->
   Result<(), EsiError>``, and ``fn encode_outputs(&self, bits: &mut
   BitSlice<u8, Lsb0>) -> Result<(), EsiError>``. The trait shall add
   no async methods — the cyclic hot path stays synchronous.

.. req:: EsiConfigurable trait shape for preop bring-up
   :id: REQ_0531
   :status: open
   :satisfies: FEAT_0054

   The crate shall define ``EsiConfigurable: EsiDevice`` with
   ``type Assignment`` and ``async fn configure<'a>(&mut self, sub:
   &SubDevicePreOperational<'a>, a: Self::Assignment) -> Result<(),
   EsiError>``. Bring-up SDO writes (InitCmds, 0x1C12 / 0x1C13 PDO
   assignment writes) live inside the generated body of this method.

.. req:: Traits live in ethercat-esi-rt, not sonic-connector
   :id: REQ_0532
   :status: open
   :satisfies: FEAT_0054

   ``EsiDevice`` and ``EsiConfigurable`` shall live in a dedicated
   ``ethercat-esi-rt`` crate. They shall not live in
   ``sonic-connector-ethercat``, ``ethercat-hal``, or any other
   sonic-internal crate, so any ethercrab user can adopt the
   generated drivers without depending on sonic.

.. req:: Object dictionary emission is a default-off cargo feature
   :id: REQ_0533
   :status: open
   :satisfies: FEAT_0054

   Emission of the full object-dictionary table per device (as a
   ``static &[(u16, u8, DataType, &str)]`` lookup, per
   :need:`ADR_0075`) shall be gated behind a default-off
   ``object-dictionary`` cargo feature on the generated module's
   parent crate. PDOs and InitCmd writes shall remain unconditional;
   only the OD table is gated. With the feature off, generated code
   shall not pay for OD blow-up (which can reach 10–50× for OD-heavy
   devices per :need:`RISK_0010`).

.. req:: Process image access via bitvec BitSlice
   :id: REQ_0534
   :status: open
   :satisfies: FEAT_0054

   ``decode_inputs`` and ``encode_outputs`` shall operate on
   ``bitvec::slice::BitSlice<u8, Lsb0>`` references covering the
   device's portion of the EtherCAT cycle PDI. The trait shall not
   embed an opinion on how the surrounding application acquires
   those slices.

Build helper
~~~~~~~~~~~~

.. feat:: Build helper (build.rs glue)
   :id: FEAT_0055
   :status: open
   :satisfies: FEAT_0050

   A trivial helper crate so downstream consumers run codegen with
   one ``build.rs`` invocation and one ``include!`` line.

.. req:: Builder API shape
   :id: REQ_0540
   :status: open
   :satisfies: FEAT_0055

   ``ethercat-esi-build`` shall expose
   ``Builder::new().glob(<pattern>).backend(<backend>).out_file(<name>).build()``
   returning ``Result<(), BuildError>``. The ``backend`` parameter
   shall be generic over ``CodegenBackend`` per :need:`REQ_0510`.

.. req:: Output written to OUT_DIR
   :id: REQ_0541
   :status: open
   :satisfies: FEAT_0055

   The helper shall write the generated module to
   ``$OUT_DIR/<out_file>`` so consumers wire it in with
   ``include!(concat!(env!("OUT_DIR"), "/<out_file>"));``.

.. req:: Cargo rerun-if directives emitted per ESI input
   :id: REQ_0542
   :status: open
   :satisfies: FEAT_0055

   The helper shall print ``cargo:rerun-if-changed=<path>`` for each
   ESI file matched by the glob and for the build script itself, so
   cargo re-runs codegen exactly when an input changes — not on every
   build.

.. req:: Generated output passes through prettyplease
   :id: REQ_0543
   :status: open
   :satisfies: FEAT_0055

   Before writing the output, the helper shall format the
   ``TokenStream`` via ``prettyplease::unparse`` so the file is
   human-readable when diffed or inspected through ``cargo expand``.

CLI inspection
~~~~~~~~~~~~~~

.. feat:: CLI inspection (cargo subcommand)
   :id: FEAT_0056
   :status: open
   :satisfies: FEAT_0050

   A ``cargo`` subcommand so users can inspect what was generated
   for a given ESI file without going through ``$OUT_DIR`` /
   ``cargo expand``. Adds discoverability with one extra crate, no
   change to the codegen path.

.. req:: cargo esi expand emits one device's generated code
   :id: REQ_0550
   :status: open
   :satisfies: FEAT_0056

   ``ethercat-esi-cli`` shall expose a ``cargo esi expand --device
   <ident>`` subcommand that parses the matching ESI file(s) and
   prints the generated module for that device to stdout, formatted
   per :need:`REQ_0543`.

.. req:: cargo esi list enumerates devices in a glob
   :id: REQ_0551
   :status: open
   :satisfies: FEAT_0056

   ``cargo esi list`` shall accept a glob pattern (defaulting to
   ``esi/*.xml`` when invoked from a crate root) and print the
   ``(ident, vendor_id, product_id, revision)`` tuple for every
   device found.

.. req:: CLI shares the parser and codegen crates
   :id: REQ_0552
   :status: open
   :satisfies: FEAT_0056

   The CLI shall depend on ``ethercat-esi`` and
   ``ethercat-esi-codegen-ethercrab`` as library dependencies. It
   shall not duplicate parse or emit logic. Output produced by the
   CLI for a given input shall be byte-identical to the output
   produced by ``ethercat-esi-build`` for the same input and
   formatter settings (:need:`REQ_0543`).

EEPROM diff verification
~~~~~~~~~~~~~~~~~~~~~~~~

.. feat:: EEPROM diff verification
   :id: FEAT_0057
   :status: open
   :satisfies: FEAT_0050

   A CI-friendly cross-check: parse an ESI XML file and the matching
   captured SII EEPROM ``.bin`` dump from a real device, then diff
   the two on identity, PDO map, and mailbox configuration. Catches
   the "vendor shipped a buggy ESI file" failure class at build time
   rather than during cyclic operation.

.. req:: Verifier ingests ESI XML plus SII binary
   :id: REQ_0560
   :status: open
   :satisfies: FEAT_0057

   ``ethercat-esi-verify`` shall expose
   ``fn verify(xml: &str, sii: &[u8]) -> Result<VerifyReport,
   VerifyError>`` that parses both inputs and compares them on:
   ``Identity`` (vendor / product / revision), the assigned PDO
   index list per direction, and the mailbox bootstrap configuration
   (CoE/EoE/FoE supported sets).

.. req:: Diagnostic output names the differing field
   :id: REQ_0561
   :status: open
   :satisfies: FEAT_0057

   When a verification fails, the ``VerifyReport`` shall name each
   differing field with both the ESI-side and SII-side values
   (e.g. ``Identity.revision: esi=0x00100000 sii=0x00110000``)
   rather than reporting only "mismatch".

.. req:: Verifier reuses the parser
   :id: REQ_0562
   :status: open
   :satisfies: FEAT_0057

   The verifier shall consume the same ``EsiFile`` IR produced by
   :need:`FEAT_0051` and shall not maintain a second parse path. SII
   binary decoding lives inside the verifier crate (no reuse from
   ethercrab — the verifier shall not depend on ethercrab per
   :need:`REQ_0520`).

.. req:: Verifier exits non-zero on mismatch
   :id: REQ_0563
   :status: open
   :satisfies: FEAT_0057

   When invoked as a binary (``ethercat-esi-verify <xml> <sii>``),
   the verifier shall exit ``0`` on match, ``1`` on any field
   mismatch, and ``2`` on parse or I/O errors. CI gates may then
   ``cargo run -p ethercat-esi-verify -- ...`` as a pre-merge
   check.

----

Anti-goals
----------

The following requirements are explicitly **rejected** — captured for
the record so future readers see what the toolchain deliberately does
not do, and why. Each rejected requirement ``:satisfies:``
:need:`FEAT_0050` to keep the umbrella's traceability complete.

.. req:: NO CAN / CANopen / EDS support in this round
   :id: REQ_0590
   :status: rejected
   :satisfies: FEAT_0050

   The toolchain shall **not** include a CAN parser, a CANopen
   runtime trait, an EDS / XDD reader, or a SocketCAN backend.
   CANopen and EtherCAT's CoE share the Object Dictionary
   semantics, but transport semantics diverge (cyclic PDI vs
   event-driven frames). A follow-on spec extracts a shared
   ``fieldbus-od-core`` IR once a concrete CANopen device is in
   scope; see :need:`ADR_0073`.

.. req:: NO proc-macro front-end
   :id: REQ_0591
   :status: rejected
   :satisfies: FEAT_0050

   The toolchain shall **not** offer an ``esi_device!("EL3001.xml")``
   proc-macro form. The IDE-discoverability gain does not justify
   the doubled codegen surface or the worse compile-time profile
   for what is effectively a one-time generation step per device set.
   ``cargo esi expand`` (:need:`REQ_0550`) covers the inspection
   use case.

.. req:: NO unification of EtherCAT and CANopen runtime traits
   :id: REQ_0592
   :status: rejected
   :satisfies: FEAT_0050

   When CANopen support is added in a follow-on spec, the runtime
   trait family shall **not** be merged with ``EsiDevice`` /
   ``EsiConfigurable``. EtherCAT's cyclic-bit-buffer model and
   CANopen's event-driven-frame model are different transport
   semantics; forcing them into one trait would leak a fake
   "process image" into CANopen and mis-model event-triggered
   TPDOs.

.. req:: NO runtime XML parsing
   :id: REQ_0593
   :status: rejected
   :satisfies: FEAT_0050

   The toolchain shall **not** parse ESI XML at application
   runtime. All XML parsing happens at build time in
   ``ethercat-esi-build`` or in the CLI tools. Consumers of the
   generated modules shall not need to ship XML files alongside
   their binary.

.. req:: NO modification of sonic-connector-ethercat runtime
   :id: REQ_0594
   :status: rejected
   :satisfies: FEAT_0050

   This spec shall **not** require any change to the runtime
   contracts of :need:`FEAT_0041` "EtherCAT reference connector".
   The connector consumes ``EsiDevice`` through a thin adapter (see
   :need:`BB_0066`); it does not become aware of XML or codegen.

.. req:: NO automatic vendor library scraping
   :id: REQ_0595
   :status: rejected
   :satisfies: FEAT_0050

   The toolchain shall **not** download, scrape, or otherwise
   fetch ESI XML from vendor websites or update servers. ESI files
   are inputs the user drops into a ``esi/`` directory; provenance
   is the user's responsibility.

----

Cross-cutting traceability
--------------------------

Every requirement on this page (excluding rejected anti-goals)
carries a ``:satisfies:`` link to its capability-cluster feat; every
cluster feat ``:satisfies:`` :need:`FEAT_0050`. Architectural
specifications refining these requirements are emitted in
:doc:`../architecture/device-codegen`. Verification artefacts are
emitted in :doc:`../verification/device-codegen`.

.. needtable::
   :types: feat
   :filter: id >= "FEAT_0050" and id <= "FEAT_0059"
   :columns: id, title, status, satisfies
   :show_filters:

.. needtable::
   :types: req
   :filter: id >= "REQ_0500" and id <= "REQ_0599"
   :columns: id, title, status, satisfies
   :show_filters:
