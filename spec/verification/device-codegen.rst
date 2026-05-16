Device-driver codegen — verification
====================================

Test cases verifying the device-driver codegen toolchain. Each
``test`` directive ``:verifies:`` one or more requirements from
:doc:`../requirements/device-codegen` (or building blocks from
:doc:`../architecture/device-codegen`).

The toolchain is build-time only — there are no cyclic-runtime
integration tests beyond what :doc:`connector` already covers for
:need:`FEAT_0041`. The verification surface here is therefore
heavier on snapshot / golden-file / property tests than on
multi-process integration.

----

Parser unit tests
-----------------

Per-crate, no I/O beyond test fixtures, parallel-safe. Live under
``crates/ethercat-esi/tests/``.

.. test:: parse() accepts a representative Beckhoff EL3001 ESI
   :id: TEST_0400
   :status: open
   :verifies: REQ_0500, REQ_0504

   Loads a canonical Beckhoff ``EL3001`` ESI XML fixture from
   ``crates/ethercat-esi/tests/fixtures/``, calls
   ``parse(xml)``, and asserts the resulting ``EsiFile`` exposes
   the expected identity (``vendor_id = 0x2``, ``product_id =
   0x0BB93052``), one device with two sync managers, the
   "Standard" and "Compact" PDO alternatives, the expected
   InitCmd count, and a non-empty object dictionary.

.. test:: Parser compiles under no_std + alloc
   :id: TEST_0401
   :status: open
   :verifies: REQ_0501

   Compile-only test: a small bin target inside
   ``crates/ethercat-esi/tests/no_std/`` declares ``#![no_std]``,
   uses ``alloc::string::String``, and parses a fixture. The
   target must compile with ``--no-default-features`` against the
   crate; the test passes if compilation succeeds (no runtime
   assertion).

.. test:: Parser is independent of ethercrab
   :id: TEST_0402
   :status: open
   :verifies: REQ_0503

   ``cargo tree -p ethercat-esi --no-default-features --target
   <host>`` shall not list ``ethercrab`` anywhere in the
   resolved graph. Implemented as a CI shell check that greps
   the output and fails on match.

.. test:: Vendor-specific elements survive as RawXml
   :id: TEST_0403
   :status: open
   :verifies: REQ_0505

   Fixture file with a fabricated ``<Vendor:UnknownElement
   foo="bar">inner</Vendor:UnknownElement>`` inside a
   ``<Device>``. After parsing, the IR carries one ``RawXml``
   entry with ``name = "Vendor:UnknownElement"``, ``attributes =
   {"foo": "bar"}``, ``inner_text = "inner"``. The parse does
   **not** return an error.

.. test:: Parse errors carry line and column
   :id: TEST_0404
   :status: open
   :verifies: REQ_0506

   A deliberately malformed ESI fixture (unclosed tag at known
   coordinates) parses to ``Err(EsiError::Xml { line, column, ..
   })`` with the expected line and column. Catching this trace
   in build-time output (per :need:`REQ_0506`) is the user
   benefit.

----

Codegen / IR tests
------------------

Per-crate, snapshot-based. Live under
``crates/ethercat-esi-codegen/tests/``.

.. test:: Name sanitisation handles ESI naming edge cases
   :id: TEST_0410
   :status: open
   :verifies: REQ_0511

   Parameterised test asserting the sanitisation map for a fixed
   table of ESI product names → Rust idents: ``EL3001-0000`` →
   ``EL3001_0000``, ``EL3204 with spaces`` → ``EL3204_with_spaces``,
   leading-digit ``1234-Module`` → ``_1234_Module``, empty / pure
   punctuation → error.

.. test:: Revision collision produces distinct idents
   :id: TEST_0411
   :status: open
   :verifies: REQ_0512

   Synthetic input set containing two devices with identical
   product name but different revisions (``0x00100000`` and
   ``0x00110000``). The generated module shall contain
   ``EL3204_REV0010`` and ``EL3204_REV0011`` (or equivalent
   deterministic suffixing). Reordering the input file list shall
   produce the same idents in the same definitions (assert via
   string comparison of generated source).

.. test:: PDO entry dedup collapses structurally identical layouts
   :id: TEST_0412
   :status: open
   :verifies: REQ_0513

   Two synthetic devices whose RxPDO entries have identical
   ``(field order, bit lengths, data types)`` produce a single
   shared PDO entry struct in the generated module; the two
   device structs reference it by name. Asserted by counting
   distinct ``PdoEntry``-bearing struct definitions in the
   ``TokenStream`` (expect 1, not 2).

.. test:: TokenStream emission, not string formatting
   :id: TEST_0413
   :status: open
   :verifies: REQ_0514

   White-box test inside ``ethercat-esi-codegen`` asserts that
   ``emit_device`` and ``emit_module_root`` return
   ``TokenStream`` values directly (compile-time check via the
   return type). A complementary lint forbids ``format!`` /
   ``write!`` / ``writeln!`` invocations within the codegen
   crate's source.

----

ethercrab backend snapshot tests
--------------------------------

Live under ``crates/ethercat-esi-codegen-ethercrab/tests/``.

.. test:: EL3001 backend output snapshot
   :id: TEST_0420
   :status: open
   :verifies: REQ_0521, REQ_0522, REQ_0523, REQ_0524

   Run parse → codegen → backend → prettyplease on the canonical
   ``EL3001`` ESI fixture. Compare the formatted output against a
   committed ``snapshots/el3001.rs`` golden file using
   ``insta::assert_snapshot!``. Reviewer regenerates the golden
   when intentional changes land; CI fails on unintentional
   churn.

.. test:: Generated registry covers every emitted device
   :id: TEST_0421
   :status: open
   :verifies: REQ_0525

   For an input set with N devices, the generated module's
   ``registry!()`` expansion contains exactly N entries mapping
   ``SubDeviceIdentity`` → factory closure. White-box test
   parses the generated output and counts entries.

.. test:: Generated module compiles under no_std + alloc
   :id: TEST_0422
   :status: open
   :verifies: REQ_0526

   A test crate at
   ``crates/ethercat-esi-codegen-ethercrab/tests/no_std_consumer/``
   has ``#![no_std]`` and ``extern crate alloc;``, ``include!``s
   the generated module from a fixed input set, and compiles
   successfully. Catches any accidental ``std::`` qualified path
   in the backend's emit code.

.. test:: Backend is the sole ethercrab consumer in the toolchain
   :id: TEST_0423
   :status: open
   :verifies: REQ_0520

   CI shell check: ``cargo tree`` invocations for
   ``ethercat-esi``, ``ethercat-esi-codegen``,
   ``ethercat-esi-build``, ``ethercat-esi-cli``, and
   ``ethercat-esi-verify`` must none of them list ``ethercrab``
   in the dependency graph. ``ethercat-esi-codegen-ethercrab``
   and ``ethercat-esi-rt`` are the only crates where
   ``ethercrab`` is allowed.

.. test:: Object-dictionary emission gated by feature flag
   :id: TEST_0424
   :status: open
   :verifies: REQ_0533

   Build the ``no_std_consumer`` test crate twice: once without
   features (the generated module's OD table is empty / absent;
   no symbol named ``OD`` exists) and once with
   ``--features object-dictionary`` (the OD ``static`` exists
   and has the expected entry count for the input set). Compares
   the two binaries' rodata sections — the no-feature build is
   smaller by an amount approximating the OD table size.

----

Runtime trait surface tests
---------------------------

Live under ``crates/ethercat-esi-rt/tests/``.

.. test:: EsiDevice trait shape compiles for a hand-written device
   :id: TEST_0430
   :status: open
   :verifies: REQ_0530

   Hand-written test impl of ``EsiDevice`` for a minimal
   ``MockDevice`` validates the trait surface compiles
   end-to-end. Asserts ``IDENTITY``, ``input_len``,
   ``output_len`` return the expected values, and that
   ``decode_inputs`` / ``encode_outputs`` round-trip a synthetic
   ``BitSlice<u8, Lsb0>``.

.. test:: EsiConfigurable async trait shape compiles
   :id: TEST_0431
   :status: open
   :verifies: REQ_0531

   Compile-only test: a mock device implements
   ``EsiConfigurable`` with ``type Assignment = MockAssignment``
   and an ``async fn configure`` body. The test passes if
   compilation succeeds; the async signature shape catches the
   trait-method-async constraint.

.. test:: ethercat-esi-rt is the trait home, not sonic-internal
   :id: TEST_0432
   :status: open
   :verifies: REQ_0532

   CI shell check: ``rg "trait EsiDevice"`` across the workspace
   matches exactly one source location, inside
   ``crates/ethercat-esi-rt/src/``. Same check for
   ``trait EsiConfigurable``.

----

Build helper tests
------------------

Live under ``crates/ethercat-esi-build/tests/``.

.. test:: Builder writes a parseable Rust file to OUT_DIR
   :id: TEST_0440
   :status: open
   :verifies: REQ_0540, REQ_0541

   Test crate driven by a fixture ESI set runs
   ``Builder::new().glob(...).backend(default).out_file(
   "devices.rs").build()`` in a ``tempfile``-backed ``OUT_DIR``.
   Asserts the file exists, is non-empty, and parses with
   ``syn::parse_file`` (catches malformed token streams).

.. test:: cargo rerun-if-changed emitted per ESI input
   :id: TEST_0441
   :status: open
   :verifies: REQ_0542

   Capture ``Builder::build()``'s stdout output (the build
   helper prints to stdout per cargo conventions). For an
   N-file glob, assert exactly N + 1 ``cargo:rerun-if-changed=``
   lines are present (one per ESI file + one for the build
   script itself).

.. test:: Output passes prettyplease formatting
   :id: TEST_0442
   :status: open
   :verifies: REQ_0543

   The generated ``devices.rs`` file is line-wrapped (no line
   exceeds 100 chars without justification), and re-running
   ``prettyplease::unparse`` on the file produces a
   byte-identical output (idempotent formatter pass).

----

CLI tests
---------

Live under ``crates/ethercat-esi-cli/tests/``.

.. test:: cargo esi expand emits a single device's code
   :id: TEST_0450
   :status: open
   :verifies: REQ_0550

   Spawn the CLI as ``cargo esi expand --device EL3001 --glob
   <fixtures>/*.xml``, capture stdout, assert the output is
   non-empty, parses as Rust, and contains exactly one ``pub
   struct EL3001`` definition.

.. test:: cargo esi list enumerates devices
   :id: TEST_0451
   :status: open
   :verifies: REQ_0551

   Run ``cargo esi list --glob <fixtures>/*.xml`` over a
   3-device fixture set, assert stdout contains 3 lines each
   matching the ``<ident>\t<vendor_id>\t<product_id>\t<revision>``
   format.

.. test:: CLI output matches build helper output byte-for-byte
   :id: TEST_0452
   :status: open
   :verifies: REQ_0552

   For one fixed device, capture the CLI's ``expand`` output and
   the build helper's per-device slice of ``$OUT_DIR/devices.rs``.
   Assert the two byte-strings are identical (catches divergent
   code paths between CLI and build).

----

EEPROM verifier tests
---------------------

Live under ``crates/ethercat-esi-verify/tests/``.

.. test:: Verifier passes on matching ESI + SII pair
   :id: TEST_0460
   :status: open
   :verifies: REQ_0560

   Use the captured pair from
   ``crates/ethercat-eeprom-dump/dumps/EL3001/`` (ESI XML +
   matching SII ``.bin``), call
   ``verify(xml, sii)``, assert ``Ok(VerifyReport {
   matched: true, .. })``.

.. test:: Verifier reports the differing field
   :id: TEST_0461
   :status: open
   :verifies: REQ_0561

   Synthetic mismatched pair: parse a real ESI but flip one bit
   in a captured SII to alter the revision field. The returned
   ``VerifyReport`` shall contain at least one ``Difference``
   entry whose ``field`` is exactly ``"Identity.revision"`` and
   whose ``esi`` / ``sii`` values match the originals.

.. test:: Verifier reuses ethercat-esi parser
   :id: TEST_0462
   :status: open
   :verifies: REQ_0562

   White-box: ``cargo tree -p ethercat-esi-verify`` lists
   ``ethercat-esi`` as a direct dependency and does **not** list
   ``ethercrab`` anywhere in the graph (re-affirming
   :need:`TEST_0423` from the verifier side).

.. test:: Verifier exit codes follow the documented matrix
   :id: TEST_0463
   :status: open
   :verifies: REQ_0563

   Spawn the verifier binary three times: matching pair (expect
   exit 0), mismatched pair (expect 1), unreadable SII path
   (expect 2). Asserted via ``Command::status().code()``.

----

Cross-cutting reproducibility tests
-----------------------------------

Verify the build-time determinism quality goal
(:need:`QG_0010`).

.. test:: Repeated codegen runs produce byte-identical output
   :id: TEST_0470
   :status: open
   :verifies: QG_0010, REQ_0543

   Run ``Builder::build()`` twice on the same input set in
   freshly-prepared ``OUT_DIR`` directories. Compare the two
   ``devices.rs`` files with ``sha256``. Assert identical.

.. test:: Input-file ordering does not affect output
   :id: TEST_0471
   :status: open
   :verifies: QG_0010, REQ_0512, REQ_0513

   Same input set, glob returns files in two different orders
   (force the order via explicit ``Builder::file(path)``
   calls). The two ``devices.rs`` outputs are byte-identical
   (catches HashMap-iteration-order nondeterminism in dedup or
   collision-handling).

.. test:: Layering integrity check (Cargo.toml audit)
   :id: TEST_0472
   :status: open
   :verifies: QG_0011, REQ_0503, REQ_0520

   CI shell check that walks each toolchain crate's
   ``Cargo.toml`` and asserts the allowed-dependency matrix:

   * ``ethercat-esi``: no ``ethercrab``, no ``proc-macro2``, no
     ``quote``, no codegen crate.
   * ``ethercat-esi-codegen``: no ``ethercrab``.
   * ``ethercat-esi-build``: no ``ethercrab`` (transitively via
     ``ethercat-esi-codegen``-only path).
   * ``ethercat-esi-verify``: no ``ethercrab``.

   Implemented with ``cargo metadata`` + ``jq``; runs in the
   workspace CI job.

----

Cross-cutting traceability
--------------------------

.. needtable::
   :types: test
   :filter: id >= "TEST_0400" and id <= "TEST_0499"
   :columns: id, title, status, verifies
   :show_filters:
