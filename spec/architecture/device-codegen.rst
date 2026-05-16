Device-driver codegen — architecture (arc42)
============================================

Architecture documentation for the device-driver codegen toolchain
(see :doc:`../requirements/device-codegen`), structured per the arc42
template and encoded with sphinx-needs using the useblocks "x-as-code"
arc42 conventions.

Each architectural element ``:refines:`` or ``:implements:`` a parent
requirement so the trace is preserved end-to-end.

.. contents:: Sections
   :local:
   :depth: 1

----

1. Introduction and goals
-------------------------

The toolchain's reason-to-exist is **build-time monomorphisation of
fieldbus device drivers**: vendor-supplied ESI XML files describe each
EtherCAT device's PDOs, mailbox, and OD; we want strongly-typed
``decode_inputs`` / ``encode_outputs`` code per device, with zero XML
at runtime and no hand-written boilerplate per terminal.

Quality goals capture the qualities the architecture is optimised for.

.. quality-goal:: Build-time determinism (same ESI in → same code out)
   :id: QG_0010
   :status: open
   :refines: FEAT_0050

   The same set of ESI inputs (modulo file ordering) shall produce a
   byte-identical generated module across machines, toolchain
   versions, and clock walls. Generation order, hash-map iteration
   order, and timestamp inclusion are explicitly excluded as sources
   of nondeterminism. Required so the generated file is reviewable
   in diffs and cachable in CI.

.. quality-goal:: Layering integrity (strict left-to-right deps)
   :id: QG_0011
   :status: open
   :refines: FEAT_0050

   Each crate in the toolchain shall depend only on crates to its
   left in the parse → codegen → tooling chain (see
   :need:`ARCH_0050`). Crossover dependencies — e.g. the parser
   reaching for ethercrab types, the build helper bypassing the
   codegen layer — are rejected. The layering is the design;
   collapsing it deletes its value.

.. quality-goal:: Zero runtime cost of codegen presence
   :id: QG_0012
   :status: open
   :refines: FEAT_0050

   A consumer of the generated modules shall pay no runtime cost
   for the codegen layer's existence: no XML parse, no allocation
   for OD tables when ``object-dictionary`` is off, no ``Box<dyn>``
   indirection on the cyclic hot path, no string IDs at runtime.

.. quality-goal:: Trait stability for ecosystem adoption
   :id: QG_0013
   :status: open
   :refines: FEAT_0054

   The ``EsiDevice`` / ``EsiConfigurable`` pair (per
   :need:`REQ_0530` / :need:`REQ_0531`) is the contract any
   ethercrab user would pivot on if generated drivers became
   community-shared. Breaking changes shall be rare and
   well-publicised; additive evolution (default methods, new
   associated types with defaults) is preferred over rewrites.

----

2. Constraints
--------------

.. constraint:: cargo build-script semantics
   :id: CON_0010
   :status: open
   :refines: FEAT_0055

   The build helper shall live within cargo's ``build.rs``
   contract: no network access, no writes outside ``OUT_DIR``,
   ``cargo:rerun-if-changed`` directives for each input file, and
   no reliance on cargo features being on or off in a way that
   varies across consumers (per :need:`REQ_0542`).

.. constraint:: ethercrab API surface as upstream
   :id: CON_0011
   :status: open
   :refines: FEAT_0053

   The ethercrab backend shall target the ethercrab API exposed by
   ``ethercrab = "0.7"`` (or whichever version sonic-connector-ethercat
   pins, per :need:`BB_0030`-style pinning). ``SubDevicePreOperational``,
   ``SubDeviceIdentity``, and the SDO write helpers are the
   contact surface; the backend shall not depend on private types.

.. constraint:: bitvec for process-image access
   :id: CON_0012
   :status: open
   :refines: FEAT_0054

   Generated ``decode_inputs`` / ``encode_outputs`` shall operate
   on ``bitvec::slice::BitSlice<u8, Lsb0>``, matching the
   :need:`REQ_0326` / :need:`REQ_0327` PDI access pattern already
   in use by ``sonic-connector-ethercat``. A second bit-slice
   abstraction shall not be introduced.

.. constraint:: no_std + alloc baseline for parser and runtime trait
   :id: CON_0013
   :status: open
   :refines: FEAT_0051

   ``ethercat-esi`` and ``ethercat-esi-rt`` shall be ``#![no_std]``
   with an ``alloc`` dependency. Build helpers, CLI, and verifier
   may depend on ``std``.

.. constraint:: ETG owns the ESI XML schema
   :id: CON_0014
   :status: open
   :refines: FEAT_0051

   The ESI schema (``EtherCATInfo.xsd``) is published by the
   EtherCAT Technology Group, not by this project. The parser
   shall track the published schema; schema drift across vendors
   shall be handled as captured in :need:`ADR_0074` (opaque-blob
   policy for unknown elements) rather than by hard-failing the
   parse.

----

3. Context and scope
--------------------

.. architecture:: Toolchain layering (crate dependency graph)
   :id: ARCH_0050
   :status: open
   :refines: FEAT_0050

   Four layers, strict left-to-right dependency. Each crate has one
   job and depends only on crates to its left. The
   ``sonic-connector-ethercat`` consumer (:need:`FEAT_0041`) sits
   to the right of the runtime trait crate and is unaware of XML
   or codegen.

   .. mermaid::

      graph LR
        subgraph Parse["1. Parse layer"]
          P["ethercat-esi<br/>XML → typed IR"]
        end
        subgraph Gen["2. Codegen layer"]
          G["ethercat-esi-codegen<br/>IR → TokenStream"]
          B["ethercat-esi-codegen-ethercrab<br/>concrete backend"]
        end
        subgraph RT["3. Runtime trait"]
          RTC["ethercat-esi-rt<br/>EsiDevice / EsiConfigurable"]
        end
        subgraph Tool["4. Tooling layer"]
          BR["ethercat-esi-build<br/>build.rs glue"]
          CLI["ethercat-esi-cli<br/>cargo esi expand / list"]
          VER["ethercat-esi-verify<br/>diff ESI vs SII .bin"]
        end
        subgraph Cons["Consumers"]
          USER["any ethercrab user<br/>(includes generated code)"]
          SCE["sonic-connector-ethercat<br/>thin EsiDevice adapter"]
        end
        P --> G
        G --> B
        B --> RTC
        B --> BR
        B --> CLI
        P --> VER
        RTC --> BR
        BR --> USER
        USER --> SCE

.. architecture:: Build-time vs runtime separation
   :id: ARCH_0051
   :status: open
   :refines: FEAT_0050

   The toolchain runs entirely at build time. Runtime consumers
   only see the generated module and link against
   ``ethercat-esi-rt`` for the trait definitions; they do not
   depend on ``ethercat-esi``, ``ethercat-esi-codegen``, or any
   tooling crate. This is the structural guarantee behind
   :need:`QG_0012` and :need:`REQ_0593`.

----

4. Solution strategy
--------------------

The toolchain's shape is the consequence of eight architectural
decisions. Each is captured as an ADR that ``:refines:`` the
requirement or feature it answers.

.. arch-decision:: Parser separated from codegen (strict layering)
   :id: ADR_0070
   :status: open
   :refines: FEAT_0050

   **Context.** A monolithic "ESI to ethercrab driver" crate would
   conflate three concerns — XML parsing, IR shape and naming
   policy, and ethercrab-specific code emission. Downstream tools
   that need only one of these (a TwinCAT importer, a network
   configurator, the EEPROM verifier) would be forced to compile
   ethercrab and ``quote!`` machinery.

   **Decision.** Three crates: ``ethercat-esi`` (parser, no_std),
   ``ethercat-esi-codegen`` (IR + backend trait, no ethercrab),
   ``ethercat-esi-codegen-ethercrab`` (concrete backend). The
   parser owns no_std purity; the codegen owns naming and
   collision policy; the backend owns ethercrab opinion.

   **Consequences.** ✅ Each layer has one job and one
   reason-to-change. ✅ Future backends (soem, minimal, no_std-only)
   are net-additive — no parser or codegen churn.
   ❌ Three crates instead of one — more ``Cargo.toml`` surface
   for the eventual maintainer. Mitigated by the workspace being
   single-repo.

.. arch-decision:: Two-trait runtime split (EsiDevice + EsiConfigurable)
   :id: ADR_0071
   :status: open
   :refines: FEAT_0054

   **Context.** The hot path (cyclic ``decode_inputs`` /
   ``encode_outputs``) is synchronous and called from the
   cycle-loop. The bring-up path (``configure``) is async, issues
   SDO writes through the ethercrab API, and only runs during
   preop. Mixing them under one trait would force either: (a) an
   ``async fn`` on the hot-path methods (forbidden, wrong
   semantics), or (b) a sync ``configure`` that can't talk to
   ethercrab's async SDO writes.

   **Decision.** Two traits. ``EsiDevice`` is sync, mandatory,
   owns the hot path. ``EsiConfigurable: EsiDevice`` is async,
   optional (a device without configurable PDO mappings can skip
   it), owns the preop SDO sequence.

   **Consequences.** ✅ Hot path stays sync and zero-cost.
   ✅ Devices that don't need preop SDO writes don't need to
   implement the async trait. ❌ Two trait names instead of one;
   the consumer-side dispatcher has to handle both shapes.
   Acceptable cost; the generated code shoulders most of the
   burden.

.. arch-decision:: PDO assignment alternatives as sum types
   :id: ADR_0072
   :status: open
   :refines: FEAT_0053

   **Context.** An ESI device commonly declares 2–4 PDO
   assignment alternatives (e.g. "Standard 16-bit", "Compact
   8-bit"). One representation in Rust is a single struct with
   ``Option<…>`` fields for each alternative's PDO entries —
   convenient but lossy: invalid combinations (two alternatives
   enabled at once) are representable.

   **Decision.** Emit an enum ``<Device>PdoAssignment`` with one
   variant per alternative, plus one ``<Device>Pdo<Variant>``
   struct per variant. The device struct's ``pdo`` field is the
   enum; selecting a variant chooses both the assignment
   bitfields and the typed PDO struct.

   **Consequences.** ✅ "Two alternatives at once" is
   unrepresentable. ✅ Per-variant PDO structs have ``Default``
   and ``Clone``; switching alternatives at runtime is a
   ``self.pdo = …`` assignment. ❌ Slightly more generated code
   per device (one enum + N structs instead of one struct).
   Negligible at the device counts in scope (<100).

.. arch-decision:: Future CANopen support via shared OD IR
   :id: ADR_0073
   :status: open
   :refines: FEAT_0050

   **Context.** EtherCAT's CoE inherits the CANopen Object
   Dictionary (CiA 301) wholesale: index/subindex addressing,
   PDO mapping objects, the data-type table. EDS (CANopen) and
   ESI (EtherCAT) describe overlapping object dictionaries.

   **Decision.** When CANopen support is in scope, the OD IR is
   extracted to a shared ``fieldbus-od-core`` crate. ``ethercat-esi``
   shrinks to "ESI-specific elements + reference to OD core".
   ``canopen-eds`` (new) parses CiA-306 INI and emits the same OD
   IR. The runtime traits are **not** merged (per :need:`REQ_0592`)
   — CANopen's event-driven transport gets its own ``CanOpenDevice``
   family in a separate crate.

   **Consequences.** ✅ ~70 % of code is shared at the IR level
   when CANopen lands. ✅ Each transport keeps its honest runtime
   shape. ❌ Future refactor required to lift OD types out of
   ``ethercat-esi``. Manageable — the lift is mechanical because
   parser and IR are already decoupled (per :need:`ADR_0070`).

.. arch-decision:: Vendor extensions captured as opaque blobs
   :id: ADR_0074
   :status: open
   :refines: FEAT_0051

   **Context.** Vendor-specific ``<Vendor:Foo>`` elements appear
   in real-world ESI files (notably Beckhoff). Three policies are
   possible: hard-fail on unknown elements (rejects most real
   files), warn (reports but parses), capture as opaque blobs
   (parses, retains the data, defers interpretation).

   **Decision.** Capture as opaque blobs. The IR carries a
   ``RawXml { name, attributes, inner }`` for every unrecognised
   element. The codegen layer ignores them; downstream tools
   (e.g. a Beckhoff-specific importer) can interpret them.

   **Consequences.** ✅ Real-world ESI files parse without
   bespoke patches per vendor. ✅ Information is preserved, not
   discarded. ❌ The IR carries a payload nobody on the
   ethercrab-backend side reads. Negligible cost; the alternative
   (parse-and-discard) is worse.

.. arch-decision:: Object dictionary as static table, feature-gated
   :id: ADR_0075
   :status: open
   :refines: FEAT_0054

   **Context.** OD-heavy devices (e.g. Beckhoff ELxxx coupling
   modules with 200+ entries) can balloon generated code by
   10–50× if every OD entry becomes a match arm. Three options:
   match arms (large, fast lookup), static table (smaller, linear
   lookup but O(log n) with binary search on sorted index),
   no emission (smallest, fails the "OD-aware diagnostics" use
   case).

   **Decision.** Emit OD entries as a sorted
   ``static OD: &[(u16, u8, DataType, &str)]`` table, gated
   behind a default-off ``object-dictionary`` cargo feature on
   the generated module's parent crate.

   **Consequences.** ✅ Default builds carry zero OD-related
   code or rodata. ✅ Diagnostic tools that need OD lookup
   ``cargo build --features object-dictionary`` and binary-search
   the sorted slice. ❌ OD-aware code is a separate compile
   profile; CI must cover both.

.. arch-decision:: Use prettyplease, not rustfmt, for emit formatting
   :id: ADR_0076
   :status: open
   :refines: FEAT_0055

   **Context.** Emitting raw ``TokenStream`` produces single-line
   files that are unreadable when inspected. Two formatting
   options: shell out to ``rustfmt`` (requires a working rustfmt
   binary at build time; cargo doesn't guarantee one) or call
   ``prettyplease::unparse`` (a library; works offline; smaller
   formatter than rustfmt).

   **Decision.** ``prettyplease`` in-process. No shell-out to
   rustfmt.

   **Consequences.** ✅ Build helper has no external command
   dependency. ✅ Output is stable across rustfmt versions.
   ❌ Slightly different formatting than the rest of the
   workspace's rustfmt-formatted code (prettyplease is opinionated
   but not 100 % rustfmt-compatible). Acceptable — generated
   files are not human-edited.

.. arch-decision:: cargo subcommand for inspection, not proc-macro
   :id: ADR_0077
   :status: open
   :refines: FEAT_0056

   **Context.** Two inspection mechanisms were on the table.
   Option A: an ``esi_device!("EL3001.xml")`` proc-macro that
   inserts the device's generated code at call sites — gives the
   IDE rust-analyzer hover info at the call site, doubles the
   codegen path. Option B: a ``cargo esi expand`` subcommand that
   prints generated code to stdout — no IDE integration, one
   codegen path.

   **Decision.** Cargo subcommand only. Proc-macro form is
   rejected per :need:`REQ_0591`.

   **Consequences.** ✅ One codegen path; tests run once. ✅ No
   proc-macro compile-time cost on every workspace ``cargo
   build``. ❌ IDE doesn't surface generated symbols on hover.
   Acceptable; the generated file is a regular file in
   ``$OUT_DIR`` that rust-analyzer indexes.

----

5. Building block view
----------------------

.. building-block:: ethercat-esi (parser crate)
   :id: BB_0060
   :status: open
   :implements: FEAT_0051

   The parse crate. Reads ESI XML via ``quick-xml`` +
   ``serde``, emits ``EsiFile`` IR. ``no_std`` + ``alloc``. Public
   API is ``pub fn parse(xml: &str) -> Result<EsiFile, EsiError>``
   and the ``EsiFile`` / ``Device`` / ``Pdo`` / ``DictEntry``
   types per :need:`REQ_0504`. Carries no dependency on
   ``ethercrab``, ``proc-macro2``, or any codegen crate.

.. building-block:: ethercat-esi-codegen (IR + backend trait)
   :id: BB_0061
   :status: open
   :implements: FEAT_0052

   Codegen layer. Owns the ``CodegenBackend`` trait
   (:need:`REQ_0510`), naming sanitisation (:need:`REQ_0511`),
   revision-disambiguation (:need:`REQ_0512`), and PDO entry
   deduplication (:need:`REQ_0513`). Depends on ``ethercat-esi``
   (left) and ``proc-macro2`` / ``quote`` / ``prettyplease``
   (right). Does not depend on ``ethercrab``.

.. building-block:: ethercat-esi-codegen-ethercrab (concrete backend)
   :id: BB_0062
   :status: open
   :implements: FEAT_0053

   The one concrete backend shipped in this round. Emits per-device
   structs implementing ``EsiDevice`` and (where the device has
   configurable PDO mappings) ``EsiConfigurable``. Sole crate in
   the toolchain that depends on ``ethercrab`` (:need:`REQ_0520`).

.. building-block:: ethercat-esi-rt (runtime trait crate)
   :id: BB_0063
   :status: open
   :implements: FEAT_0054

   The minimal trait crate consumed by generated devices and
   adapters. Owns ``EsiDevice``, ``EsiConfigurable``,
   ``SubDeviceIdentity``, ``EsiError``. Depends on ``ethercrab``
   (for ``SubDevicePreOperational``) and ``bitvec``. ``no_std`` +
   ``alloc``. Deliberately thin so the contract is small.

.. building-block:: ethercat-esi-build (build.rs glue)
   :id: BB_0064
   :status: open
   :implements: FEAT_0055

   Build-script helper consumed by downstream crates from their
   ``build.rs``. One method: ``Builder::new().glob(...).backend(...)
   .out_file(...).build()``. Wires parse → codegen → prettyplease
   → write to ``$OUT_DIR``. Emits ``cargo:rerun-if-changed`` per
   :need:`REQ_0542`.

.. building-block:: ethercat-esi-cli (cargo subcommand)
   :id: BB_0065
   :status: open
   :implements: FEAT_0056

   Cargo subcommand binary providing ``cargo esi expand`` and
   ``cargo esi list``. Pulls in ``ethercat-esi`` and
   ``ethercat-esi-codegen-ethercrab`` as library deps, formats
   output with ``prettyplease`` (re-using :need:`REQ_0543`).
   Binary lives outside any build script — invoked by the user,
   not by cargo on every build.

.. building-block:: ethercat-esi-verify (EEPROM diff tool)
   :id: BB_0066
   :status: open
   :implements: FEAT_0057

   Cross-validates ESI XML against captured SII EEPROM ``.bin``
   dumps. Standalone binary plus library API
   (``fn verify(xml: &str, sii: &[u8]) -> Result<VerifyReport,
   VerifyError>``). Depends on ``ethercat-esi`` only; the SII
   decoder lives in this crate to keep :need:`REQ_0520` honest.

.. building-block:: sonic-connector-ethercat EsiDevice adapter
   :id: BB_0067
   :status: open
   :implements: FEAT_0050

   The thin glue inside ``sonic-connector-ethercat`` (:need:`BB_0030`
   neighbourhood) that maps any ``EsiDevice`` into whatever
   internal device-trait the connector consumes. Written once, not
   touched per terminal addition. Concrete shape is local to the
   connector crate and out of scope for this spec; this BB exists
   to record the adapter as the *only* place where the codegen
   toolchain touches the runtime connector.

----

6. Runtime view
---------------

.. architecture:: Build-time generation flow
   :id: ARCH_0052
   :status: open
   :refines: FEAT_0055

   The build-time codegen sequence when a downstream crate's
   ``build.rs`` runs.

   .. mermaid::

      sequenceDiagram
        participant Cargo as cargo
        participant Build as build.rs
        participant Esi as ethercat-esi
        participant Codegen as ethercat-esi-codegen
        participant Backend as -codegen-ethercrab
        participant PP as prettyplease
        participant Out as $OUT_DIR/devices.rs

        Cargo->>Build: invoke build.rs
        Build->>Esi: parse(xml) for each ESI file
        Esi-->>Build: EsiFile IR
        Build->>Codegen: generate(IR, backend)
        Codegen->>Backend: emit_device(d) per device
        Backend-->>Codegen: TokenStream
        Codegen->>Backend: emit_module_root(devices)
        Backend-->>Codegen: TokenStream (with registry!())
        Codegen-->>Build: combined TokenStream
        Build->>PP: unparse(tokenstream)
        PP-->>Build: formatted source
        Build->>Out: write devices.rs
        Build-->>Cargo: cargo:rerun-if-changed=esi/*.xml

.. architecture:: Preop bring-up flow (per device)
   :id: ARCH_0053
   :status: open
   :refines: FEAT_0054

   The runtime sequence when a generated device's ``configure``
   method runs during the EtherCAT bus's PRE-OP → SAFE-OP
   transition (per :need:`REQ_0315`).

   .. mermaid::

      sequenceDiagram
        participant App as application
        participant Dev as <Device>
        participant Sub as SubDevicePreOperational
        participant Bus as EtherCAT bus

        App->>Dev: Device::default()
        App->>Dev: configure(&sub, Assignment::Standard).await
        Dev->>Sub: sdo_write(0x1C12, 0, 0) (clear)
        Sub->>Bus: SDO download
        loop per RxPDO in alternative
          Dev->>Sub: sdo_write(0x1C12, idx, pdo_index)
          Sub->>Bus: SDO download
        end
        Dev->>Sub: sdo_write(0x1C12, 0, N) (commit count)
        Dev->>Sub: sdo_write(0x1C13, 0, 0)
        loop per TxPDO in alternative
          Dev->>Sub: sdo_write(0x1C13, idx, pdo_index)
        end
        Dev->>Sub: sdo_write(0x1C13, 0, M)
        loop per InitCmd in ESI mailbox section
          Dev->>Sub: sdo_write(initcmd.index, initcmd.subindex, initcmd.data)
        end
        Dev-->>App: Ok(())
        Note over Sub,Bus: caller transitions PRE-OP → SAFE-OP

----

7. Deployment view
------------------

.. architecture:: Toolchain crate placement in workspace
   :id: ARCH_0054
   :status: open
   :refines: FEAT_0050

   All seven toolchain crates live in ``crates/`` alongside the
   existing ``sonic-connector-ethercat`` and friends. The
   workspace ``Cargo.toml`` adds them to ``members``; pinning
   matches the rest of the workspace (``rust-toolchain.toml`` MSRV
   1.85, edition 2024 per :need:`CON_0003`-style constraint
   tracking).

   No deployment-time changes: the toolchain is a build-time
   artefact. The only runtime consequence is that
   ``sonic-connector-ethercat`` gains a path-dep on
   ``ethercat-esi-rt`` and an internal ``EsiDevice`` adapter.

----

8. Crosscutting concepts
------------------------

The crosscutting axes are owned by section 1 (quality goals) and
section 2 (constraints). The two persistent runtime concepts —
the ``EsiDevice`` trait and the ``SubDeviceIdentity`` const — both
live in :need:`BB_0063` and are referenced from generated code,
adapters, and dispatch registries alike. They are the contract
the rest of the toolchain orbits.

----

9. Architectural decisions
--------------------------

All decisions are captured in section 4 (Solution strategy) as ADR
records :need:`ADR_0070` through :need:`ADR_0077`. This section is
deliberately a pointer rather than a duplicate — arc42's
recommendation when decisions are dense in the solution strategy
narrative.

----

10. Quality requirements
------------------------

The quality goals in section 1 (:need:`QG_0010` through
:need:`QG_0013`) define the qualities. The verification artefacts
in :doc:`../verification/device-codegen` exercise each one.

----

11. Risks and technical debt
----------------------------

.. risk:: OD table size blow-up on coupling modules
   :id: RISK_0010
   :status: open
   :links: ADR_0075, REQ_0533

   Beckhoff coupling modules can declare 200+ OD entries. With
   ``object-dictionary`` enabled, the static OD table per coupler
   reaches ~10 KB of rodata. Mitigated by the feature flag
   (:need:`ADR_0075`); becomes a tracked debt if a downstream
   consumer enables the feature and ships to constrained MCU
   targets. Not yet a problem in the current sonic deployment
   (Linux gateway only — :need:`REQ_0325`).

.. risk:: Beckhoff vendor extensions churn the IR
   :id: RISK_0011
   :status: open
   :links: ADR_0074, REQ_0505

   Beckhoff ships ESI files with ``<Vendor:Foo>`` elements that
   evolve between TwinCAT releases. Opaque-blob capture
   (:need:`ADR_0074`) keeps the parser stable, but downstream
   importers that interpret vendor blobs will need version
   awareness. Mitigation: keep vendor-blob interpretation in
   per-vendor importer crates, not in the parser or backend.

.. risk:: ethercrab API churn breaking the backend
   :id: RISK_0012
   :status: open
   :links: CON_0011, BB_0062

   ``ethercrab`` is pre-1.0 and its API has evolved
   (SubDevice / MainDevice rename, async signature changes). A
   minor-version bump can require a backend re-emit. Mitigation:
   pin ethercrab in ``ethercat-esi-codegen-ethercrab``'s
   ``Cargo.toml`` to the same range as
   ``sonic-connector-ethercat``; bump in lockstep.

.. risk:: ESI XML schema drift across vendors
   :id: RISK_0013
   :status: open
   :links: CON_0014, REQ_0505

   Wago, Omron, and Beckhoff have shipped ESI files at different
   schema-version baselines. The parser shall track the highest
   shipped schema with the opaque-blob escape hatch
   (:need:`ADR_0074`) catching everything else. A schema-only
   conformance test set (:need:`TEST_0420`) anchors the parser
   against the canonical schema; a real-world fixture set
   (:need:`TEST_0421`) anchors it against actual vendor files.

.. risk:: Generated code becomes load-bearing without migration path
   :id: RISK_0014
   :status: open
   :links: QG_0013

   If many consumers depend on the generated module's struct
   names (e.g. ``EL3001 { pdo: … }``), changing naming policy
   (:need:`REQ_0511`) becomes a breaking change for every
   downstream. Mitigation: lock naming policy under
   ``ethercat-esi-codegen`` (not the backend), version-bump that
   crate per semver on any naming change, document the breaking
   matrix in CHANGELOG.

----

12. Glossary
------------

.. term:: ESI
   :id: GLOSS_0020
   :status: open

   EtherCAT Slave Information — an XML file describing a single
   EtherCAT device's identity, PDOs, mailbox, distributed clocks,
   and object dictionary. Schema is published by ETG
   (``EtherCATInfo.xsd``).

.. term:: SII
   :id: GLOSS_0021
   :status: open

   Slave Information Interface — the on-device EEPROM that
   carries a binary subset of the ESI data, readable over the
   EtherCAT bus by the master. ``ethercat-esi-verify`` cross-checks
   ESI XML against captured SII ``.bin`` dumps.

.. term:: PDO
   :id: GLOSS_0022
   :status: open

   Process Data Object — a fixed-length packed set of OD entries
   exchanged on every EtherCAT cycle. RxPDO = master → device
   (outputs); TxPDO = device → master (inputs).

.. term:: CoE
   :id: GLOSS_0023
   :status: open

   CANopen over EtherCAT — mailbox protocol carrying CANopen SDO
   writes (e.g. PDO assignment writes to 0x1C12 / 0x1C13).

.. term:: OD (Object Dictionary)
   :id: GLOSS_0024
   :status: open

   The indexed (16-bit index + 8-bit sub-index) catalogue of
   readable / writable objects on a CANopen or CoE device.
   Inherited by EtherCAT from CANopen (CiA 301).

.. term:: InitCmd
   :id: GLOSS_0025
   :status: open

   An SDO write sequence declared inside an ESI ``<Mailbox>``
   section that must run during a specific state transition
   (typically PRE-OP → SAFE-OP). Carries the bring-up data
   (filter coefficients, scaling values, channel modes) the
   device expects before cyclic operation.
