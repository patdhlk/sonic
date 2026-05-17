CANopen device-driver codegen
=============================

This page captures the requirements for the **CANopen device-driver
codegen toolchain**: a layered set of crates that translates CANopen
Electronic Data Sheet (EDS, CiA 306) files into strongly-typed Rust
driver modules at build time, with zero runtime INI parsing and no
dependency on the ``sonic-connector-can`` runtime.

The decomposition is the peer of :doc:`device-codegen` for CANopen,
executing the lift foreseen by :need:`ADR_0073` (now closed by
:need:`ADR_0078`):

* **Top-level umbrella feature** — :need:`FEAT_0060` — peer to
  :need:`FEAT_0050` (EtherCAT codegen). The umbrella is build-time
  only and orthogonal to :need:`FEAT_0046` "CAN reference connector";
  the runtime adapter that wires generated devices into the connector
  is a follow-on spec.
* **Shared OD core** — :need:`FEAT_0061` lifts the OD IR (Identity,
  DictEntry, DataType, PdoEntry, PdoMap, AccessRights) out of
  ``ethercat-esi`` into a new ``fieldbus-od-core`` crate so both
  parsers share it.
* **Capability-cluster sub-features** — one per crate-layer concern,
  each ``:satisfies:`` :need:`FEAT_0060`.
* **Requirements** — concrete shall-clauses that ``:satisfies:`` a
  capability-cluster feature.

This round covers EDS only. DCF (Device Configuration File) support
and a live-bus verifier are explicitly out of scope; the architecture
preserves the option to add either later (see anti-goals).

Top-level umbrella
------------------

.. feat:: CANopen device-driver codegen toolchain
   :id: FEAT_0060
   :status: open

   A layered set of Rust crates that consumes CANopen EDS files (CiA
   306) and emits strongly-typed driver modules at build time. The
   toolchain is organised as five layers that depend only leftwards:

   1. **Shared OD core** — ``fieldbus-od-core``: OD IR lifted from
      ``ethercat-esi``. ``no_std`` + ``alloc``. Knows no XML, no INI,
      no transport.
   2. **Parse layer** — ``canopen-eds``: CiA 306 INI → typed IR.
      Depends on ``fieldbus-od-core``. No codegen, no transport dep.
   3. **Codegen layer** — ``canopen-eds-codegen`` (IR →
      ``TokenStream`` via ``CodegenBackend`` trait) plus
      ``canopen-eds-codegen-sonic`` (the one concrete backend this
      round, targeting the ``CanOpenDevice`` trait surface).
   4. **Runtime trait crate** — ``canopen-eds-rt``: the
      ``CanOpenDevice`` / ``CanOpenConfigurable`` traits the
      generated drivers implement. Frame-per-PDO dispatch — no
      cyclic process-image model (per :need:`REQ_0592`).
   5. **Tooling layer** — ``canopen-eds-build`` (build.rs glue),
      ``canopen-eds-cli`` (``cargo eds expand`` / ``cargo eds list``
      one-shot tools), and ``canopen-eds-verify`` (offline diff of
      EDS XML against a captured SDO-upload JSON dump).

   The ``sonic-connector-can`` crate (see :need:`FEAT_0046`) is not
   part of this toolchain. A thin adapter that maps any
   ``CanOpenDevice`` into the connector's frame plumbing is a
   follow-on spec; this umbrella does not require changes to
   :need:`FEAT_0046`'s runtime contracts (see :need:`REQ_0795`).

----

Capability clusters
-------------------

The umbrella decomposes into eight capability clusters. Each cluster
is a sub-feature ``:satisfies:`` :need:`FEAT_0060`, with concrete
shall-clauses underneath.

Shared OD core
~~~~~~~~~~~~~~

.. feat:: Shared OD core
   :id: FEAT_0061
   :status: open
   :satisfies: FEAT_0060

   A new crate ``fieldbus-od-core`` carrying the OD types both ESI
   and EDS parsers need (CiA 301 semantics). Lifted out of
   ``ethercat-esi`` so both fieldbuses parse against the same IR.
   Executes the lift foreseen by :need:`ADR_0073`.

.. req:: No transport-specific types in fieldbus-od-core
   :id: REQ_0700
   :status: open
   :satisfies: FEAT_0061

   ``fieldbus-od-core`` shall declare no transport-specific types.
   The crate shall not name ``ethercrab``, ``socketcan``,
   ``sonic_connector_*``, or any I/O-bearing crate as a dependency.

.. req:: no_std + alloc, no mandatory serde
   :id: REQ_0701
   :status: open
   :satisfies: FEAT_0061

   The crate shall be ``#![no_std]`` with an ``alloc`` dependency.
   No ``serde``, no ``quick-xml``, no ``serde-ini`` in the default
   feature set. Type derives (``Serialize``, ``Deserialize``,
   ``Hash``) shall sit behind opt-in cargo features so embedded
   consumers do not pay for them.

.. req:: OD type surface
   :id: REQ_0702
   :status: open
   :satisfies: FEAT_0061

   The crate shall carry ``Identity`` (``vendor_id``,
   ``product_code``, ``revision`` — all ``u32``), ``DataType``
   (enumerating the CiA 301 data-type table), ``AccessRights``
   (``Const`` / ``ReadOnly`` / ``WriteOnly`` / ``ReadWrite``),
   ``DictEntry`` (index, sub_index, name, data_type, access,
   default/min/max bytes), ``PdoEntry`` (index, sub_index, bit_len,
   optional name), and ``PdoMap`` (assigned-to OD index plus entry
   list).

.. req:: ethercat-esi re-exports lifted types
   :id: REQ_0703
   :status: open
   :satisfies: FEAT_0061

   ``ethercat-esi`` shall re-export ``Identity``, ``DataType``,
   ``AccessRights``, ``DictEntry``, ``PdoEntry``, and ``PdoMap``
   from ``fieldbus-od-core`` so existing :need:`FEAT_0050`-era
   consumers compile source-unchanged. The re-export façade shall
   stay in place permanently — it is not deprecated.

.. req:: canopen-eds uses fieldbus-od-core types
   :id: REQ_0704
   :status: open
   :satisfies: FEAT_0061

   ``canopen-eds`` shall use ``fieldbus-od-core`` types for every
   OD-shaped field in its IR. The crate shall not redefine
   ``Identity``, ``DictEntry``, ``PdoEntry``, or ``PdoMap``
   locally.

EDS parser
~~~~~~~~~~

.. feat:: EDS parser
   :id: FEAT_0062
   :status: open
   :satisfies: FEAT_0060

   A pure parser crate. Reads CiA 306 INI, emits a typed IR rooted
   in :need:`FEAT_0061` types. Knows nothing about codegen,
   transports, or sonic-internal crates. Suitable for any downstream
   tool — codegen, network configurator, simulator, verifier.

.. req:: Pure parse function with no I/O
   :id: REQ_0720
   :status: open
   :satisfies: FEAT_0062

   The crate shall expose ``parse(text: &str) -> Result<EdsFile,
   EdsError>``. The function shall perform no filesystem or network
   I/O; the caller is responsible for reading the EDS bytes.

.. req:: no_std + alloc, no upstream coupling
   :id: REQ_0721
   :status: open
   :satisfies: FEAT_0062

   The crate shall be ``#![no_std]`` with ``alloc``, and shall not
   depend on ``ethercrab``, ``canopen-eds-codegen``,
   ``sonic-connector-can``, or any transport crate. A downstream
   tool that only needs the IR shall not be forced to compile the
   codegen layer.

.. req:: serde-derive INI backend
   :id: REQ_0722
   :status: open
   :satisfies: FEAT_0062

   The crate shall implement parsing on top of a serde INI
   deserialiser (``serde_ini`` is the primary candidate; the
   alternative ``rust-ini`` is acceptable behind the same façade).
   Hand-written line parsing is rejected — schema maintenance lives
   in the ``serde`` derives.

.. req:: Parse errors carry line and column
   :id: REQ_0723
   :status: open
   :satisfies: FEAT_0062

   ``EdsError`` variants raised during parsing shall carry the
   source line and byte column of the offending construct so
   build-time diagnostics point at the failing EDS file location.

.. req:: Unknown sections captured as RawSection
   :id: REQ_0724
   :status: open
   :satisfies: FEAT_0062

   Unknown EDS sections shall be retained in the IR as
   ``RawSection { name, keys: Vec<(String, String)> }``. The parser
   shall not hard-fail on unknown sections — direct CANopen
   analogue of the EtherCAT ``RawXml`` policy (:need:`ADR_0074`).

.. req:: Liberal parsing — warn and continue on quirks
   :id: REQ_0725
   :status: open
   :satisfies: FEAT_0062

   The parser shall be liberal on common EDS formatting quirks:
   BOM, CRLF/LF mix, trailing whitespace on values, comments after
   values (``; ...``), redundant whitespace around ``=``, and EDS
   exporter-quirk keys (e.g. ``LineFeed``). Quirks shall surface
   as ``EdsFile::warnings: Vec<EdsWarning>`` carrying ``{ line,
   kind }`` so build-time logs can print them without failing the
   parse. A strict mode is not provided in this round.

.. req:: IR carries identity, OD, PDO comm + maps
   :id: REQ_0726
   :status: open
   :satisfies: FEAT_0062

   The IR shall represent, per device: ``Identity`` (lifted from
   the ``[Identity]`` block / OD index ``0x1018``), ``DeviceInfo``
   (vendor / product names, baud-rate flags, NMT-boot-slave flag),
   the OD as ``Vec<DictEntry>``, ``Vec<PdoMap>`` for declared
   RPDOs / TPDOs, and parallel ``Vec<RPdoComm>`` / ``Vec<TPdoComm>``
   for PDO communication parameters (transmission type, cob-id,
   inhibit time, event timer).

Codegen IR and backend trait
~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. feat:: Codegen IR and backend trait
   :id: FEAT_0063
   :status: open
   :satisfies: FEAT_0060

   The codegen-side IR (an extension of the parser IR with naming /
   collision policy applied) and the ``CodegenBackend`` trait that
   lets multiple emitters share that IR. This crate
   (``canopen-eds-codegen``) knows nothing about INI and nothing
   about sonic-connector-can.

.. req:: CodegenBackend trait shape
   :id: REQ_0730
   :status: open
   :satisfies: FEAT_0063

   The crate shall define a ``CodegenBackend`` trait with
   ``fn emit_device(&self, device: &eds::Device) -> Result<TokenStream,
   CodegenError>`` and ``fn emit_module_root(&self, devices: &[eds::Device])
   -> Result<TokenStream, CodegenError>``. The top-level entry point
   shall be ``fn generate<B: CodegenBackend>(eds: &EdsFile, backend:
   &B) -> Result<TokenStream, CodegenError>``.

.. req:: Naming policy is owned by codegen, not the backend
   :id: REQ_0731
   :status: open
   :satisfies: FEAT_0063

   The ``canopen-eds-codegen`` crate shall sanitise EDS
   ``ProductName`` strings into valid Rust identifiers (whitespace,
   hyphens, slashes, dots → ``_``; leading digit prefixed with
   ``_``) before invoking the backend. Backends shall receive
   idents pre-validated; they shall not re-implement sanitisation.

.. req:: Revision collision handled deterministically
   :id: REQ_0732
   :status: open
   :satisfies: FEAT_0063

   When two EDS files share ``ProductName`` but differ in
   ``RevisionNumber`` (OD index ``0x1018:03``), the codegen layer
   shall disambiguate using a deterministic ``_REV<hex>`` suffix
   derived from the raw ``RevisionNumber``. Input file order shall
   not affect the generated idents.

.. req:: Common PDO entry types deduplicated
   :id: REQ_0733
   :status: open
   :satisfies: FEAT_0063

   When two or more devices' PDOs include structurally identical
   entry layouts (same bit-len + data-type tuple list), the codegen
   layer shall emit one shared PDO entry struct referenced by both
   devices rather than two duplicated structs. Structural equality
   is the dedup key; field names do not need to match.

.. req:: Emission target is proc_macro2 TokenStream
   :id: REQ_0734
   :status: open
   :satisfies: FEAT_0063

   The codegen layer shall produce ``proc_macro2::TokenStream``
   values and assemble them with ``quote!``. String-templated
   emission (``format!`` + write) is rejected — token-level
   construction preserves span / hygiene and yields rustfmt-able
   output via ``prettyplease`` (per :need:`ADR_0076`).

.. req:: One EDS file equals one device
   :id: REQ_0735
   :status: open
   :satisfies: FEAT_0063

   The codegen layer shall treat one EDS file as one device.
   ``emit_module_root`` shall accept ``&[EdsFile]`` and emit one
   device per file plus the shared registry table per
   :need:`REQ_0745`.

sonic-connector-can backend
~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. feat:: sonic-connector-can codegen backend
   :id: FEAT_0064
   :status: open
   :satisfies: FEAT_0060

   The opinionated, concrete backend that emits per-device structs
   implementing the runtime traits in :need:`FEAT_0065`. The only
   crate in the toolchain that depends on ``canopen-eds-rt``.

.. req:: Backend crate is the sole canopen-eds-rt dependency
   :id: REQ_0740
   :status: open
   :satisfies: FEAT_0064

   ``canopen-eds-codegen-sonic`` shall be the only crate in the
   toolchain that declares ``canopen-eds-rt`` as a dependency.
   Neither ``canopen-eds``, ``canopen-eds-codegen``,
   ``canopen-eds-build``, ``canopen-eds-cli``, nor
   ``canopen-eds-verify`` shall depend on ``canopen-eds-rt``.

.. req:: One device struct per EDS file
   :id: REQ_0741
   :status: open
   :satisfies: FEAT_0064

   For each parsed EDS file (per :need:`REQ_0735`), the backend
   shall emit exactly one Rust struct named per the sanitised
   product ident (per :need:`REQ_0731` and :need:`REQ_0732`),
   deriving ``Debug + Default + Clone``.

.. req:: Identity const emitted per device
   :id: REQ_0742
   :status: open
   :satisfies: FEAT_0064

   For each generated device struct, the backend shall emit an
   accompanying ``pub const <IDENT>_IDENTITY: Identity = Identity {
   vendor_id, product_code, revision };`` so identity-driven
   dispatch (per :need:`REQ_0745`) can use a static table.

.. req:: PDO declarations emitted as sum types
   :id: REQ_0743
   :status: open
   :satisfies: FEAT_0064

   For each declared PDO mapping in the EDS, the backend shall
   emit one variant of ``<IDENT>Rpdo`` / ``<IDENT>Tpdo`` enum plus
   one payload struct per variant. Modelling PDOs as nullable
   fields on the device struct is rejected — every declared PDO is
   a closed, named choice.

.. req:: Dummy entries skipped in PDO payload structs
   :id: REQ_0744
   :status: open
   :satisfies: FEAT_0064

   For each PDO mapping, the backend shall skip CANopen ``Dummy*``
   data-type entries when emitting payload struct fields — only
   real mapped objects appear as fields. Bit offsets of real
   fields shall be threaded through generated ``decode`` /
   ``encode`` bodies (not carried as named padding fields). See
   :need:`ADR_0083`.

.. req:: Generated module root exposes a registry
   :id: REQ_0745
   :status: open
   :satisfies: FEAT_0064

   The module root emitted by ``emit_module_root`` shall expose a
   ``registry!()`` declarative macro (or equivalent generated
   ``static`` table) mapping each emitted device's ``Identity`` to
   a factory closure returning ``Box<dyn CanOpenDevice>``.
   Identity-based dispatch in a downstream adapter shall be
   reducible to a ``HashMap`` lookup against this table.

.. req:: Bring-up SDO writes emitted from EDS
   :id: REQ_0746
   :status: open
   :satisfies: FEAT_0064

   ``impl CanOpenConfigurable for <IDENT>`` shall emit SDO writes
   for: PDO communication parameters (OD ranges
   ``0x1400..0x14FF`` and ``0x1800..0x18FF``), PDO mapping
   parameters (``0x1600..0x17FF`` and ``0x1A00..0x1BFF``), and the
   ``[DeviceInfo].NMT_BootSlave``-driven NMT start request when
   declared. Values shall come from the EDS — per-bus customisation
   is DCF territory, out of scope per :need:`REQ_0790`.

.. req:: Object dictionary emission is a default-off cargo feature
   :id: REQ_0747
   :status: open
   :satisfies: FEAT_0064

   Emission of the full OD table per device (as a sorted
   ``static OD: &[(u16, u8, DataType, &str)]`` lookup, per
   :need:`ADR_0075`) shall be gated behind a default-off
   ``object-dictionary`` cargo feature on the generated module's
   parent crate. PDOs and bring-up SDO writes shall remain
   unconditional; only the OD table is gated.

.. req:: Generated code compiles under no_std + alloc
   :id: REQ_0748
   :status: open
   :satisfies: FEAT_0064

   The emitted device modules shall compile under ``#![no_std]`` +
   ``alloc``. The backend shall not emit ``std::``-qualified paths
   in generated code.

Runtime trait surface
~~~~~~~~~~~~~~~~~~~~~

.. feat:: Runtime trait surface
   :id: FEAT_0065
   :status: open
   :satisfies: FEAT_0060

   The minimal trait pair the generated devices implement and any
   downstream CAN adapter consumes. Lives in a tiny
   ``canopen-eds-rt`` crate so the runtime contract is not coupled
   to either the codegen or the connector.

.. req:: CanOpenDevice trait shape
   :id: REQ_0750
   :status: open
   :satisfies: FEAT_0065

   The crate shall define a ``CanOpenDevice`` trait with
   ``const IDENTITY: Identity``, ``fn node_id(&self) -> u8``,
   ``fn set_node_id(&mut self, id: u8)``,
   ``fn nmt_state(&self) -> NmtState``,
   ``fn set_nmt_state(&mut self, s: NmtState)``,
   ``fn on_rpdo(&mut self, idx: u8, frame: PdoFrame<'_>) -> Result<(), CanOpenError>``,
   and
   ``fn drain_tpdos(&mut self, out: &mut dyn TpdoSink) -> Result<(), CanOpenError>``.
   The trait shall add no async methods on the RPDO/TPDO path —
   the frame-handling hot path stays synchronous.

.. req:: CanOpenConfigurable trait shape for bring-up
   :id: REQ_0751
   :status: open
   :satisfies: FEAT_0065

   The crate shall define
   ``CanOpenConfigurable: CanOpenDevice`` carrying
   ``async fn configure<S: SdoClient>(&mut self, sdo: &mut S) ->
   Result<(), CanOpenError>``. Bring-up SDO writes (PDO comm,
   PDO mapping, optional NMT start) live inside the generated
   body of this method.

.. req:: Traits live in canopen-eds-rt, not sonic-connector-can
   :id: REQ_0752
   :status: open
   :satisfies: FEAT_0065

   ``CanOpenDevice`` and ``CanOpenConfigurable`` shall live in a
   dedicated ``canopen-eds-rt`` crate. They shall not live in
   ``sonic-connector-can``, ``fieldbus-od-core``, or any other
   sonic-internal crate. Any CAN consumer shall be able to adopt
   the generated drivers without depending on sonic.

.. req:: Frame payloads use heapless::Vec<u8, 8>
   :id: REQ_0753
   :status: open
   :satisfies: FEAT_0065

   ``PdoFrame`` shall borrow an inbound payload as ``&[u8]``;
   ``PdoOut`` shall carry the outbound payload as
   ``heapless::Vec<u8, 8>`` (classical CAN cap). ``PdoOut::can_id:
   u32`` shall carry the resolved COB-ID computed by generated code
   from the current ``node_id()`` and the EDS-declared base
   COB-ID; the consumer forwards it as the CAN frame ID without
   further resolution. CAN-FD payloads are out of scope this round
   (see :need:`REQ_0791`).

.. req:: Frame-per-PDO dispatch shape
   :id: REQ_0754
   :status: open
   :satisfies: FEAT_0065

   ``on_rpdo(idx, frame)`` shall accept an RPDO enumeration index
   in ``0..=3`` (CANopen's 4-RPDO cap from CiA 301) and route to
   the right typed decoder in generated code. Caller-side
   resolution from CAN ID to RPDO enumeration uses the configured
   ``0x1400..0x14FF`` communication parameters and is the
   consumer's responsibility. ``drain_tpdos(out)`` shall produce
   zero or more frames per call into a caller-provided
   ``TpdoSink``.

.. req:: CanOpenError variant surface
   :id: REQ_0755
   :status: open
   :satisfies: FEAT_0065

   ``CanOpenError`` shall enumerate CiA 301 SDO abort codes
   (``AbortCode(u32)``), payload-length mismatch
   (``PdoLenMismatch { expected: u8, got: u8 }``), unknown-index,
   ``NmtStateViolation``, and a
   ``TransportFailed(&'static str)`` variant for caller-supplied
   I/O failures.

.. req:: RPDO rejected outside Operational state
   :id: REQ_0756
   :status: open
   :satisfies: FEAT_0065

   Generated code shall reject ``on_rpdo`` calls when
   ``nmt_state() != NmtState::Operational``, returning
   ``CanOpenError::NmtStateViolation``. NMT state is
   caller-managed; the trait carries getter / setter but no
   transition methods.

Build helper
~~~~~~~~~~~~

.. feat:: Build helper (build.rs glue)
   :id: FEAT_0066
   :status: open
   :satisfies: FEAT_0060

   A trivial helper crate so downstream consumers run codegen with
   one ``build.rs`` invocation and one ``include!`` line.

.. req:: Builder API shape
   :id: REQ_0760
   :status: open
   :satisfies: FEAT_0066

   ``canopen-eds-build`` shall expose
   ``Builder::new().glob(<pattern>).backend(<backend>).out_file(<name>).build()``
   returning ``Result<(), BuildError>``. The ``backend`` parameter
   shall be generic over ``CodegenBackend`` per :need:`REQ_0730`.

.. req:: Output written to OUT_DIR
   :id: REQ_0761
   :status: open
   :satisfies: FEAT_0066

   The helper shall write the generated module to
   ``$OUT_DIR/<out_file>`` so consumers wire it in with
   ``include!(concat!(env!("OUT_DIR"), "/<out_file>"));``.

.. req:: Cargo rerun-if directives emitted per EDS input
   :id: REQ_0762
   :status: open
   :satisfies: FEAT_0066

   The helper shall print ``cargo:rerun-if-changed=<path>`` for
   each EDS file matched by the glob and for the build script
   itself, so cargo re-runs codegen exactly when an input changes —
   not on every build.

.. req:: Generated output passes through prettyplease
   :id: REQ_0763
   :status: open
   :satisfies: FEAT_0066

   Before writing the output, the helper shall format the
   ``TokenStream`` via ``prettyplease::unparse`` so the file is
   human-readable when diffed or inspected (per :need:`ADR_0076`).

.. req:: Parser warnings surface as cargo warnings
   :id: REQ_0764
   :status: open
   :satisfies: FEAT_0066

   Parser warnings raised under :need:`REQ_0725` shall surface as
   ``cargo:warning=<line>: <kind>`` lines so they appear in cargo
   build output. A strict mode that promotes warnings to errors is
   not provided in this round.

CLI inspection
~~~~~~~~~~~~~~

.. feat:: CLI inspection (cargo subcommand)
   :id: FEAT_0067
   :status: open
   :satisfies: FEAT_0060

   A ``cargo`` subcommand so users can inspect what was generated
   for a given EDS file without going through ``$OUT_DIR`` /
   ``cargo expand``. Adds discoverability with one extra crate, no
   change to the codegen path (per :need:`ADR_0077`).

.. req:: cargo eds expand emits one device's generated code
   :id: REQ_0770
   :status: open
   :satisfies: FEAT_0067

   ``canopen-eds-cli`` shall expose a
   ``cargo eds expand --device <ident>`` subcommand that parses
   the matching EDS file(s) and prints the generated module for
   that device to stdout, formatted per :need:`REQ_0763`.

.. req:: cargo eds list enumerates devices in a glob
   :id: REQ_0771
   :status: open
   :satisfies: FEAT_0067

   ``cargo eds list`` shall accept a glob pattern (defaulting to
   ``eds/*.eds`` when invoked from a crate root) and print the
   ``(ident, vendor_id, product_code, revision)`` tuple for every
   device found.

.. req:: CLI shares the parser and codegen crates
   :id: REQ_0772
   :status: open
   :satisfies: FEAT_0067

   The CLI shall depend on ``canopen-eds`` and
   ``canopen-eds-codegen-sonic`` as library dependencies. It shall
   not duplicate parse or emit logic. Output produced by the CLI
   for a given input shall be byte-identical to the output produced
   by ``canopen-eds-build`` for the same input and formatter
   settings (:need:`REQ_0763`).

EDS ↔ SDO-dump verification
~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. feat:: EDS ↔ SDO-dump verification
   :id: FEAT_0068
   :status: open
   :satisfies: FEAT_0060

   A CI-friendly cross-check: parse an EDS file and a captured
   SDO-upload JSON dump from a real node, then diff the two on
   identity, declared PDO maps, and PDO communication parameters.
   Catches the "vendor shipped a buggy EDS" failure class at build
   time rather than during cyclic operation. Offline-only this
   round — live-bus verification is out of scope per
   :need:`REQ_0797`.

.. req:: Verifier ingests EDS plus JSON SDO-dump
   :id: REQ_0780
   :status: open
   :satisfies: FEAT_0068

   ``canopen-eds-verify`` shall expose
   ``fn verify(eds: &str, dump: &SdoDump) -> Result<VerifyReport,
   VerifyError>`` that parses both inputs and compares them on:
   ``Identity`` (vendor / product / revision from OD index
   ``0x1018:01..03``), the declared PDO map index list per
   direction and the entries within each declared mapping, PDO
   communication parameters (transmission type, cob-id, event
   timer), and device-type at OD index ``0x1000``.

.. req:: Diagnostic output names the differing field
   :id: REQ_0781
   :status: open
   :satisfies: FEAT_0068

   When a verification fails, the ``VerifyReport`` shall name each
   differing field with both the EDS-side and dump-side values
   (e.g. ``Identity.product_code: eds=0x60900000 dump=0x60910000``)
   rather than reporting only "mismatch".

.. req:: Verifier reuses the parser
   :id: REQ_0782
   :status: open
   :satisfies: FEAT_0068

   The verifier shall consume the same ``EdsFile`` IR produced by
   :need:`FEAT_0062` and shall not maintain a second parse path.
   JSON SDO-dump decoding lives inside the verifier crate. The
   verifier shall not depend on ``canopen-eds-codegen``,
   ``canopen-eds-rt``, or ``sonic-connector-can``.

.. req:: Verifier exits non-zero on mismatch
   :id: REQ_0783
   :status: open
   :satisfies: FEAT_0068

   When invoked as a binary
   (``canopen-eds-verify <eds> <dump.json>``), the verifier shall
   exit ``0`` on match, ``1`` on any field mismatch, and ``2`` on
   parse or I/O errors. CI gates may then
   ``cargo run -p canopen-eds-verify -- ...`` as a pre-merge check.

.. req:: SDO-dump JSON schema versioned
   :id: REQ_0784
   :status: open
   :satisfies: FEAT_0068

   The SDO-dump file format shall be versioned via a top-level
   ``schema`` field carrying the string
   ``sonic.canopen.sdo-dump.v1``. Unknown schema strings shall be
   rejected with a parse error before any field comparison runs
   (per :need:`ADR_0086`).

----

Anti-goals
----------

The following requirements are explicitly **rejected** — captured for
the record so future readers see what the toolchain deliberately does
not do, and why. Each rejected requirement ``:satisfies:``
:need:`FEAT_0060` to keep the umbrella's traceability complete.

.. req:: NO DCF support this round
   :id: REQ_0790
   :status: rejected
   :satisfies: FEAT_0060

   The toolchain shall **not** parse DCF (Device Configuration
   File) inputs this round. EDS describes a device's *shape*; DCF
   describes per-node *configuration* (chosen RPDO/TPDO mapping,
   node-id, SDO-write-at-bringup values). DCF support is a
   follow-on spec; adding it later does not require an IR break
   because the EDS IR already carries the shape DCF references.

.. req:: NO CAN-FD payload support in PdoOut
   :id: REQ_0791
   :status: rejected
   :satisfies: FEAT_0060

   ``PdoOut::payload`` shall **not** support CAN-FD's 64-byte
   payload this round. Lifting ``heapless::Vec<u8, 8>`` to a
   const-generic capacity (``heapless::Vec<u8, N>``) is a follow-on.
   See :need:`ADR_0084`.

.. req:: NO proc-macro front-end
   :id: REQ_0792
   :status: rejected
   :satisfies: FEAT_0060

   The toolchain shall **not** offer a
   ``canopen_device!("foo.eds")`` proc-macro form. The
   IDE-discoverability gain does not justify the doubled codegen
   surface or the worse compile-time profile. ``cargo eds expand``
   (:need:`REQ_0770`) covers the inspection use case. Mirrors
   :need:`REQ_0591`.

.. req:: NO unification of EtherCAT and CANopen runtime traits
   :id: REQ_0793
   :status: rejected
   :satisfies: FEAT_0060

   ``CanOpenDevice`` shall **not** be merged with
   ``EsiDevice`` / ``EsiConfigurable``. EtherCAT's cyclic-bit-buffer
   model and CANopen's event-driven-frame model are different
   transport semantics; forcing them into one trait would leak a
   fake "process image" into CANopen and mis-model event-triggered
   TPDOs. This requirement closes the loop on :need:`REQ_0592` by
   delivering the separate trait family the rejection reserved.

.. req:: NO runtime EDS parsing
   :id: REQ_0794
   :status: rejected
   :satisfies: FEAT_0060

   The toolchain shall **not** parse EDS files at application
   runtime. All EDS parsing happens at build time in
   ``canopen-eds-build`` or in the CLI tools. Consumers of the
   generated modules shall not need to ship EDS files alongside
   their binary. Mirrors :need:`REQ_0593`.

.. req:: NO modification of sonic-connector-can runtime
   :id: REQ_0795
   :status: rejected
   :satisfies: FEAT_0060

   This spec shall **not** require any change to the runtime
   contracts of :need:`FEAT_0046` "CAN reference connector". A
   thin adapter that maps any ``CanOpenDevice`` into the
   connector's frame plumbing is a follow-on spec; this umbrella
   stops at producing typed devices that implement
   ``canopen-eds-rt`` traits. Mirrors :need:`REQ_0594`.

.. req:: NO automatic vendor library scraping
   :id: REQ_0796
   :status: rejected
   :satisfies: FEAT_0060

   The toolchain shall **not** download, scrape, or otherwise
   fetch EDS files from vendor websites or update servers. EDS
   files are inputs the user drops into an ``eds/`` directory;
   provenance is the user's responsibility. Mirrors
   :need:`REQ_0595`.

.. req:: NO live-bus verifier this round
   :id: REQ_0797
   :status: rejected
   :satisfies: FEAT_0060

   ``canopen-eds-verify`` shall **not** open a SocketCAN
   interface, send live SDO upload requests, or otherwise touch a
   real bus. Verification is strictly offline — EDS file vs.
   captured JSON dump (:need:`REQ_0780`). Live verification
   belongs in the follow-on ``sonic-connector-can`` adapter spec
   where the bus is already at hand.

----

Cross-cutting traceability
--------------------------

Every requirement on this page (excluding rejected anti-goals)
carries a ``:satisfies:`` link to its capability-cluster feat; every
cluster feat ``:satisfies:`` :need:`FEAT_0060`. Architectural
specifications refining these requirements are emitted in
:doc:`../architecture/canopen-codegen`. Verification artefacts are
emitted in :doc:`../verification/canopen-codegen`.

.. needtable::
   :types: feat
   :filter: id >= "FEAT_0060" and id <= "FEAT_0069"
   :columns: id, title, status, satisfies
   :show_filters:

.. needtable::
   :types: req
   :filter: id >= "REQ_0700" and id <= "REQ_0799"
   :columns: id, title, status, satisfies
   :show_filters:
