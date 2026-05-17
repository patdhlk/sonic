CANopen device-driver codegen — architecture (arc42)
====================================================

Architecture documentation for the CANopen device-driver codegen
toolchain (see :doc:`../requirements/canopen-codegen`), structured
per the arc42 template and encoded with sphinx-needs using the
useblocks "x-as-code" arc42 directive types. Mirrors the structure of
:doc:`device-codegen` so reviewers can read both umbrellas 1:1.

Each architectural element ``:refines:`` or ``:implements:`` a parent
requirement so the trace is preserved end-to-end.

.. contents:: Sections
   :local:
   :depth: 1

----

1. Introduction and goals
-------------------------

The toolchain's reason-to-exist is **build-time monomorphisation of
CANopen device drivers**: vendor-supplied EDS files describe each
node's PDOs, OD, and bring-up SDO sequence; we want strongly-typed
``on_rpdo`` / ``drain_tpdos`` code per device, with zero INI parsing
at runtime and no hand-written boilerplate per node.

Quality goals capture the qualities the architecture is optimised for.

.. quality-goal:: Build-time determinism (same EDS in → same code out)
   :id: QG_0014
   :status: open
   :refines: FEAT_0060

   The same set of EDS inputs (modulo file ordering) shall produce a
   byte-identical generated module across machines, toolchain
   versions, and clock walls. Generation order, hash-map iteration
   order, and timestamp inclusion are explicitly excluded as sources
   of nondeterminism. Required so the generated file is reviewable
   in diffs and cachable in CI.

.. quality-goal:: Layering integrity (strict left-to-right deps)
   :id: QG_0015
   :status: open
   :refines: FEAT_0060

   Each crate in the toolchain shall depend only on crates to its
   left in the OD-core → parse → codegen → tooling chain (see
   :need:`ARCH_0070`). Crossover dependencies — e.g. the parser
   reaching for ``canopen-eds-rt`` types, the build helper bypassing
   the codegen layer — are rejected. The layering is the design;
   collapsing it deletes its value.

.. quality-goal:: Zero runtime cost of codegen presence
   :id: QG_0016
   :status: open
   :refines: FEAT_0060

   A consumer of the generated modules shall pay no runtime cost
   for the codegen layer's existence: no INI parse, no allocation
   for OD tables when ``object-dictionary`` is off, no ``Box<dyn>``
   indirection on the frame hot path, no string IDs at runtime.

.. quality-goal:: Trait stability for ecosystem adoption
   :id: QG_0017
   :status: open
   :refines: FEAT_0065

   The ``CanOpenDevice`` / ``CanOpenConfigurable`` pair (per
   :need:`REQ_0750` / :need:`REQ_0751`) is the contract any CAN
   consumer would pivot on if generated drivers became
   community-shared. Breaking changes shall be rare and
   well-publicised; additive evolution (default methods, new
   associated types with defaults) is preferred over rewrites.

----

2. Constraints
--------------

.. constraint:: cargo build-script semantics
   :id: CON_0020
   :status: open
   :refines: FEAT_0066

   The build helper shall live within cargo's ``build.rs``
   contract: no network access, no writes outside ``OUT_DIR``,
   ``cargo:rerun-if-changed`` directives for each input file
   (per :need:`REQ_0762`), and no reliance on cargo features being
   on or off in a way that varies across consumers.

.. constraint:: CiA 301 / 306 own the EDS schema
   :id: CON_0021
   :status: open
   :refines: FEAT_0062

   The EDS schema is published by CAN in Automation (CiA 306) and
   the underlying OD semantics by CiA 301. The parser shall track
   the published schema; schema drift across vendors shall be
   handled as captured in :need:`REQ_0724` (unknown-section policy)
   and :need:`REQ_0725` (liberal-quirks policy) rather than by
   hard-failing the parse.

.. constraint:: no_std + alloc baseline for OD core, parser, runtime
   :id: CON_0022
   :status: open
   :refines: FEAT_0061

   ``fieldbus-od-core``, ``canopen-eds``, and ``canopen-eds-rt``
   shall be ``#![no_std]`` with an ``alloc`` dependency (per
   :need:`REQ_0701` / :need:`REQ_0721` / :need:`REQ_0748`). Build
   helpers, CLI, and verifier may depend on ``std``.

.. constraint:: heapless 0.8 surface for fixed-capacity buffers
   :id: CON_0023
   :status: open
   :refines: FEAT_0065

   ``PdoOut::payload`` shall be ``heapless::Vec<u8, 8>`` from the
   ``heapless`` 0.8 family (or whichever version sonic's workspace
   pins). The constant-8 capacity matches classical CAN's 8-byte
   payload cap; CAN-FD's 64-byte payload is deferred per
   :need:`REQ_0791` and :need:`ADR_0084`.

----

3. Context and scope
--------------------

.. architecture:: Toolchain layering (crate dependency graph)
   :id: ARCH_0070
   :status: open
   :refines: FEAT_0060

   Five layers, strict left-to-right dependency. Each crate has one
   job and depends only on crates to its left. The follow-on
   ``sonic-connector-can`` adapter would sit to the right of the
   runtime trait crate and is unaware of INI or codegen.

   .. mermaid::

      graph LR
        subgraph OD["1. Shared OD core"]
          ODC["fieldbus-od-core<br/>Identity, DictEntry,<br/>PdoEntry, PdoMap"]
        end
        subgraph Parse["2. Parse layer"]
          EE["ethercat-esi<br/>(re-exports + ESI specifics)"]
          P["canopen-eds<br/>INI → typed IR"]
        end
        subgraph Gen["3. Codegen layer"]
          G["canopen-eds-codegen<br/>IR → TokenStream"]
          B["canopen-eds-codegen-sonic<br/>concrete backend"]
        end
        subgraph RT["4. Runtime trait"]
          RTC["canopen-eds-rt<br/>CanOpenDevice / CanOpenConfigurable"]
        end
        subgraph Tool["5. Tooling layer"]
          BR["canopen-eds-build<br/>build.rs glue"]
          CLI["canopen-eds-cli<br/>cargo eds expand / list"]
          VER["canopen-eds-verify<br/>EDS ↔ SDO-dump diff"]
        end
        subgraph Cons["Consumers (follow-on)"]
          USER["any CAN consumer<br/>(includes generated code)"]
          SCC["sonic-connector-can<br/>thin CanOpenDevice adapter"]
        end
        ODC --> EE
        ODC --> P
        P --> G
        G --> B
        B --> RTC
        B --> BR
        B --> CLI
        P --> VER
        RTC --> BR
        BR --> USER
        USER --> SCC

.. architecture:: Build-time vs runtime separation
   :id: ARCH_0071
   :status: open
   :refines: FEAT_0060

   The toolchain runs entirely at build time. Runtime consumers
   only see the generated module and link against
   ``canopen-eds-rt`` for the trait definitions; they do not depend
   on ``canopen-eds``, ``canopen-eds-codegen``, or any tooling
   crate. This is the structural guarantee behind :need:`QG_0016`
   and :need:`REQ_0794`.

----

4. Building blocks (per-crate)
------------------------------

.. building-block:: fieldbus-od-core
   :id: BB_0080
   :status: open
   :implements: FEAT_0061

   The shared OD types lifted out of ``ethercat-esi``. Pure data
   carriers: ``Identity``, ``DataType`` (CiA 301 enumeration),
   ``AccessRights``, ``DictEntry``, ``PdoEntry``, ``PdoMap``. No
   I/O, no serde in the default surface, no transport dep.

.. building-block:: canopen-eds parser crate
   :id: BB_0081
   :status: open
   :implements: FEAT_0062

   INI parser front-end. ``parse(text) -> Result<EdsFile, EdsError>``.
   Uses a serde-derive backend (``serde_ini`` primary candidate).
   Liberal-quirk policy raises warnings rather than failing
   (:need:`REQ_0725`). Unknown sections retained as ``RawSection``.

.. building-block:: canopen-eds-codegen
   :id: BB_0082
   :status: open
   :implements: FEAT_0063

   Codegen IR + ``CodegenBackend`` trait. Owns naming policy
   (sanitisation, revision-suffix), structural dedup of PDO entry
   shapes, and the ``generate<B: CodegenBackend>`` entry point.

.. building-block:: canopen-eds-codegen-sonic
   :id: BB_0083
   :status: open
   :implements: FEAT_0064

   The opinionated concrete backend. Emits one device struct per
   EDS, sum-typed RPDO / TPDO enums (one variant per declared
   mapping), an ``Identity`` const per device, a module-root
   registry mapping ``Identity → factory``, and ``impl
   CanOpenConfigurable`` bodies driving bring-up SDO writes.

.. building-block:: canopen-eds-rt
   :id: BB_0084
   :status: open
   :implements: FEAT_0065

   The runtime trait crate. ``CanOpenDevice`` (sync RPDO/TPDO
   methods, identity const, NMT state getter/setter) and
   ``CanOpenConfigurable`` (async ``configure`` over a caller-
   supplied ``SdoClient``).

.. building-block:: canopen-eds-build
   :id: BB_0085
   :status: open
   :implements: FEAT_0066

   ``build.rs`` glue. Drives glob → parse → codegen → backend →
   ``prettyplease::unparse`` → write to ``$OUT_DIR``. Emits one
   ``cargo:rerun-if-changed`` per matched EDS and one for the
   build script. Surfaces parser warnings as ``cargo:warning=``
   lines.

.. building-block:: canopen-eds-cli
   :id: BB_0086
   :status: open
   :implements: FEAT_0067

   ``cargo eds expand`` / ``cargo eds list`` subcommand. Shares
   ``canopen-eds`` + ``canopen-eds-codegen-sonic`` as libraries;
   output is byte-identical to the build helper's per-device slice.

.. building-block:: canopen-eds-verify
   :id: BB_0087
   :status: open
   :implements: FEAT_0068

   Offline EDS vs JSON SDO-dump diff. Consumes the same ``EdsFile``
   IR as the codegen path; JSON dump decoding lives inside the
   verifier. ``0 / 1 / 2`` exit code on match / mismatch / error.

.. building-block:: sonic-connector-can adapter (follow-on)
   :id: BB_0088
   :status: open
   :links: FEAT_0046

   Out-of-scope for this round. A thin adapter that maps any
   ``CanOpenDevice`` into the connector's frame plumbing. Lookup
   from ``Identity`` to factory via the generated registry
   (:need:`REQ_0745`); resolution from inbound CAN ID to RPDO
   enumeration via the configured ``0x1400..0x14FF`` comm
   parameters. Tracked here so the umbrella diagram (:need:`ARCH_0070`)
   is complete; deliverable belongs to a follow-on spec
   (:need:`REQ_0795`).

----

5. Solution strategy
--------------------

The toolchain's shape is the consequence of nine architectural
decisions captured below. The structure of the decisions mirrors
:doc:`device-codegen` so the rationale chain from ESI to EDS is
explicit and visible.

.. arch-decision:: Lift OD IR to fieldbus-od-core now
   :id: ADR_0078
   :status: open
   :refines: FEAT_0061
   :links: ADR_0073

   **Context.** :need:`ADR_0073` foresaw the OD-IR lift but left it
   open. Two paths were possible: (a) lift now as part of the
   CANopen codegen round, (b) ship CANopen with a duplicated OD IR
   and lift later.

   **Decision.** Lift now. ``fieldbus-od-core`` is created as a new
   crate; ``ethercat-esi`` shrinks to "ESI-specific elements +
   re-exports from OD core"; ``canopen-eds`` parses against the same
   IR.

   **Consequences.** ✅ Parser cost amortised over both fieldbuses.
   ✅ Closes :need:`ADR_0073`. ✅ No future breaking-change cycle to
   lift later. ❌ One mechanical refactor on existing
   :need:`FEAT_0050` crates. The lift is low-risk because parser and
   IR were already decoupled (per :need:`ADR_0070`).

.. arch-decision:: fieldbus-od-core stays data-only
   :id: ADR_0079
   :status: open
   :refines: FEAT_0061

   **Context.** Possible additions to a shared OD crate include
   built-in serde derives, ``Hash`` derives, INI / XML helpers,
   proc-macro support.

   **Decision.** ``fieldbus-od-core`` is data-only, ``no_std +
   alloc``, no proc-macro. Type derives (``Serialize``,
   ``Deserialize``, ``Hash``) sit behind opt-in cargo features so
   embedded consumers don't pay for them.

   **Consequences.** ✅ Smallest possible blast radius for both
   parsers. ✅ Stable surface — adding a derive is additive. ❌ Two
   consumers (parsers) carry their own serde wiring; acceptable
   since the serde frontends are different anyway (XML vs INI).

.. arch-decision:: Re-export from ethercat-esi, do not break it
   :id: ADR_0080
   :status: open
   :refines: FEAT_0061

   **Context.** :need:`FEAT_0050` is already shipped. Two options
   for compatibility: (a) break the API and bump major version,
   (b) re-export the lifted types from ``ethercat-esi`` as a thin
   façade.

   **Decision.** Re-export. ``ethercat-esi::Identity``,
   ``ethercat-esi::DictEntry``, etc. continue to be valid paths;
   they resolve to ``fieldbus_od_core::*`` under the hood. The
   façade is permanent, not deprecated.

   **Consequences.** ✅ Existing :need:`FEAT_0050` consumers
   compile source-unchanged. ✅ No major version bump needed. ❌
   Two paths exist for the same type. Acceptable — the canonical
   path is documented (``fieldbus_od_core``) and the façade is the
   compatibility seam.

.. arch-decision:: INI backend choice — serde-derive façade
   :id: ADR_0081
   :status: open
   :refines: FEAT_0062

   **Context.** Two reasonable INI crates exist in the Rust
   ecosystem with passive (no-I/O) APIs: ``serde_ini`` and
   ``rust-ini``. Both can drive a serde-derive frontend.

   **Decision.** Treat them as interchangeable behind a serde-derive
   façade. ``serde_ini`` is the primary candidate. If the chosen
   crate cannot satisfy line/column error reporting
   (:need:`REQ_0723`), the alternative is acceptable so long as the
   serde-derive surface is preserved.

   **Consequences.** ✅ The IR is decoupled from the INI tokeniser
   choice. ✅ Backend can be flipped without IR churn. ❌ The choice
   is deferred to the planning phase; the spec does not fix it.

.. arch-decision:: PDO entry dedup is structural, name-blind
   :id: ADR_0082
   :status: open
   :refines: FEAT_0063

   **Context.** When two devices' PDOs carry the same bit-len +
   data-type tuple list but different field names (e.g.
   ``ControlWord`` vs. ``StatusWord1``), the dedup question is
   whether names matter.

   **Decision.** Structural dedup only — names are not part of the
   dedup key (per :need:`REQ_0733`). The EtherCAT side made the
   same call implicitly; this ADR captures it explicitly for both
   fieldbuses going forward.

   **Consequences.** ✅ Higher dedup hit rate across devices that
   share a CiA profile (e.g. CiA 402 servo drives). ✅ Smaller
   generated artefacts. ❌ Two devices' identical-shaped PDO
   structs may have field names from one device only. Acceptable —
   downstream code accesses by position via typed getter, not by
   the EDS-side string.

.. arch-decision:: Dummy entries skip into bit offsets, not padding fields
   :id: ADR_0083
   :status: open
   :refines: FEAT_0064

   **Context.** CANopen permits ``Dummy*`` data-type entries
   (e.g. ``DummyUInt32``) in PDO mappings to pad bit positions
   without binding a real OD object. Three modelling options:
   named padding fields (``pub _pad_0: u32``), unnamed tuple
   padding, or skip entirely.

   **Decision.** Skip. Typed PDO structs carry only real-payload
   fields; bit offsets are threaded through generated ``decode`` /
   ``encode`` bodies (per :need:`REQ_0744`). Padding bits are
   zero-initialised on encode and ignored on decode.

   **Consequences.** ✅ Cleaner API surface. ✅ No temptation for
   callers to write padding fields. ❌ ``decode`` / ``encode`` body
   complexity carries the bit-offset arithmetic; acceptable cost.

.. arch-decision:: heapless::Vec<u8, 8> for PdoOut payload
   :id: ADR_0084
   :status: open
   :refines: FEAT_0065

   **Context.** Outbound TPDO frames need a payload buffer. Three
   options: ``Vec<u8>`` (allocates per frame), fixed-array
   ``[u8; 8]`` (fixed length, can't represent shorter PDOs),
   ``heapless::Vec<u8, 8>`` (no-alloc, capacity bound, length-tracked).

   **Decision.** ``heapless::Vec<u8, 8>``. Matches classical CAN's
   8-byte payload cap; gives length-aware encode that supports
   PDOs shorter than 8 bytes; keeps the ``no_std`` story clean.

   **Consequences.** ✅ No per-frame allocation. ✅ Sound across
   embedded targets without a global allocator. ❌ CAN-FD's
   64-byte payload would need ``heapless::Vec<u8, N>`` with a
   const generic; deferred per :need:`REQ_0791`. The migration is
   mechanical: type-parameterise ``PdoOut`` over its capacity.

.. arch-decision:: Async only on configure, sync on frame path
   :id: ADR_0085
   :status: open
   :refines: FEAT_0065

   **Context.** The trait surface must distinguish hot-path frame
   plumbing (``on_rpdo``, ``drain_tpdos``) from one-shot bring-up
   (``configure``). Three options: all-sync (forces sync SDO),
   all-async (drags an executor into the cycle loop), mixed.

   **Decision.** Mixed. ``on_rpdo`` and ``drain_tpdos`` stay
   synchronous; ``configure`` is async. The caller's tokio (or
   embassy, or whatever) runtime drives bring-up; the frame hot
   path runs in whatever scheduler the consumer chooses.

   **Consequences.** ✅ No runtime dependency leaks into the hot
   path. ✅ Same posture as ``ethercat-esi-rt``'s sync decode /
   encode (per :need:`REQ_0530`). ❌ Caller must arrange an
   ``SdoClient`` impl that completes ``await`` against its CAN
   transport. Acceptable — same shape any CAN runtime adapter
   needs anyway.

.. arch-decision:: JSON SDO-dump format with versioned schema
   :id: ADR_0086
   :status: open
   :refines: FEAT_0068

   **Context.** The verifier needs a dump format to compare EDS
   against. Four options: CSV (lossy on hex / type info), custom
   binary (opaque to git review), YAML (heavier dep), JSON
   (inspectable, diff-able, schema-tag-able).

   **Decision.** JSON with explicit ``schema`` version string
   (``sonic.canopen.sdo-dump.v1``). Per :need:`REQ_0784`. Unknown
   schema strings reject before any field comparison runs.

   **Consequences.** ✅ Inspectable in git diffs. ✅ Easy to
   produce from any tool (Python ``canopen``, shell scripts over
   ``candump``, future ``sonic-connector-can`` adapter). ✅
   Versioned — schema evolution is non-breaking. ❌ One more
   serde-json dep on the verifier; trivial cost.

----

6. Risks
--------

.. risk:: EDS files in the wild are inconsistent
   :id: RISK_0020
   :status: open
   :links: REQ_0725

   Vendor EDS exporters historically produce subtly different
   dialects (LineFeed key variations, comment styles, value
   trimming). Mitigation is the liberal-parser policy
   (:need:`REQ_0725`) — warn and continue rather than reject. The
   parser will accumulate fixture exposure to known-quirky files
   over time; the warning channel makes regressions visible.

.. risk:: serde-ini ecosystem thinness
   :id: RISK_0021
   :status: open
   :links: ADR_0081, REQ_0722

   The Rust serde-INI ecosystem is less mature than serde-XML.
   Both candidate crates (``serde_ini``, ``rust-ini``) have small
   maintainer pools. Mitigation: the façade pattern in
   :need:`ADR_0081` ensures the IR survives a backend swap, and
   the parser surface (``parse(text) -> Result<EdsFile, EdsError>``)
   is small enough that a worst-case fork or replacement is
   low-effort.

.. risk:: CiA 301 OD blow-up on profile-rich devices
   :id: RISK_0022
   :status: open
   :links: REQ_0747, ADR_0075

   CiA 402 servo drives and similar profile-rich devices carry
   200+ OD entries. Generating the full OD table per device would
   balloon the codegen artefact by an order of magnitude.
   Mitigation: OD emission is feature-gated default-off
   (:need:`REQ_0747` mirroring :need:`ADR_0075`); the EtherCAT
   side has already proven this approach for OD-heavy Beckhoff
   modules (cf. :need:`RISK_0010`).

.. risk:: COB-ID base assumptions in generated code
   :id: RISK_0023
   :status: open
   :links: REQ_0753

   Generated code computes ``PdoOut::can_id`` from the current
   ``node_id()`` and the EDS-declared base COB-ID. Devices that
   deviate from CANopen's default COB-ID assignment scheme (e.g.
   manually-overridden bus layouts) would produce wrong CAN IDs.
   Mitigation: the EDS already carries the base COB-ID per PDO
   communication entry; deviations show up as a value other than
   the default. The DCF follow-on (:need:`REQ_0790`) is the right
   place to thread per-bus overrides; this round honestly inherits
   whatever the EDS declares.

----

7. Cross-cutting traceability
-----------------------------

.. needtable::
   :types: arch-decision
   :filter: id >= "ADR_0078" and id <= "ADR_0086"
   :columns: id, title, status, refines
   :show_filters:

.. needtable::
   :types: building-block
   :filter: id >= "BB_0080" and id <= "BB_0089"
   :columns: id, title, status, implements
   :show_filters:

.. needtable::
   :types: quality-goal
   :filter: id >= "QG_0014" and id <= "QG_0017"
   :columns: id, title, status, refines
   :show_filters:

.. needtable::
   :types: constraint
   :filter: id >= "CON_0020" and id <= "CON_0023"
   :columns: id, title, status, refines
   :show_filters:

.. needtable::
   :types: risk
   :filter: id >= "RISK_0020" and id <= "RISK_0023"
   :columns: id, title, status, links
   :show_filters:
