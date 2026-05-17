CANopen device-driver codegen — verification
============================================

Test cases verifying the CANopen device-driver codegen toolchain.
Each ``test`` directive ``:verifies:`` one or more requirements from
:doc:`../requirements/canopen-codegen` (or building blocks /
quality goals from :doc:`../architecture/canopen-codegen`).

The toolchain is build-time only — there are no cyclic-runtime
integration tests beyond what :doc:`connector` already covers for
:need:`FEAT_0046`. The verification surface here is therefore
heavier on snapshot / golden-file / property tests than on
multi-process integration. Mirrors the structure of
:doc:`device-codegen` so reviewers can read both verification
pages 1:1.

----

OD-core unit tests
------------------

Per-crate, no I/O beyond synthetic inputs, parallel-safe. Live
under ``crates/fieldbus-od-core/tests/``.

.. test:: Identity, DictEntry, PdoEntry round-trip
   :id: TEST_0600
   :status: open
   :verifies: REQ_0702

   Unit tests construct each public type with synthetic values,
   clone, and compare for structural equality. No I/O. Confirms the
   public surface compiles and supports the trait derives expected
   by :need:`REQ_0702`.

.. test:: fieldbus-od-core has no transport deps
   :id: TEST_0601
   :status: open
   :verifies: REQ_0700

   CI shell check: ``cargo tree -p fieldbus-od-core --no-default-features``
   shall not list ``ethercrab``, ``socketcan``, ``sonic-connector-can``,
   ``sonic-connector-ethercat``, or any I/O-bearing crate in the
   resolved graph. Fails the CI job on any match.

.. test:: fieldbus-od-core compiles under no_std + alloc
   :id: TEST_0602
   :status: open
   :verifies: REQ_0701

   Compile-only test: a small bin target inside
   ``crates/fieldbus-od-core/tests/no_std/`` declares ``#![no_std]``,
   uses ``alloc::vec::Vec``, constructs each type. Compiles with
   ``--no-default-features``; the test passes if compilation
   succeeds.

.. test:: ethercat-esi re-exports lifted types
   :id: TEST_0603
   :status: open
   :verifies: REQ_0703, REQ_0704

   Compile-only test: a test crate under
   ``crates/ethercat-esi/tests/reexport/`` writes
   ``use ethercat_esi::{Identity, DictEntry, PdoEntry, PdoMap};``
   and constructs each. Compiles if and only if the re-export
   façade is in place.

----

EDS parser tests
----------------

Per-crate, no I/O beyond test fixtures, parallel-safe. Live under
``crates/canopen-eds/tests/``.

.. test:: parse() accepts a representative Maxon EPOS4 EDS
   :id: TEST_0610
   :status: open
   :verifies: REQ_0720, REQ_0722, REQ_0726

   Loads a canonical Maxon ``EPOS4`` EDS fixture from
   ``crates/canopen-eds/tests/fixtures/``, calls ``parse(text)``,
   and asserts the resulting ``EdsFile`` exposes the expected
   identity (``vendor_id = 0x000000FB``), one device, two declared
   TPDOs and two declared RPDOs, the matching communication
   parameter records, and a non-empty object dictionary.

.. test:: Parser compiles under no_std + alloc
   :id: TEST_0611
   :status: open
   :verifies: REQ_0721

   Compile-only test: a small bin target inside
   ``crates/canopen-eds/tests/no_std/`` declares ``#![no_std]``,
   parses a fixture EDS string baked in as ``include_str!``. Must
   compile with ``--no-default-features`` against the crate.

.. test:: Parser is independent of codegen and transport
   :id: TEST_0612
   :status: open
   :verifies: REQ_0721

   CI shell check: ``cargo tree -p canopen-eds`` shall not list
   ``canopen-eds-codegen``, ``canopen-eds-rt``,
   ``sonic-connector-can``, ``socketcan``, or ``ethercrab`` anywhere
   in the resolved graph.

.. test:: Unknown sections survive as RawSection
   :id: TEST_0613
   :status: open
   :verifies: REQ_0724

   Fixture EDS containing a fabricated ``[ManufacturerSpecific_FF]``
   section with two keys. After parsing, the IR carries one
   ``RawSection`` entry whose ``name`` is exactly the section
   header and whose ``keys`` lists both key/value pairs. The parse
   does **not** return an error.

.. test:: Parse errors carry line and column
   :id: TEST_0614
   :status: open
   :verifies: REQ_0723

   A deliberately malformed EDS fixture (missing ``=`` at a known
   line / column) parses to ``Err(EdsError::Syntax { line, column,
   .. })`` with the expected values. Catching this trace in
   build-time output is the user benefit.

.. test:: Liberal-quirk parsing emits warnings without failing
   :id: TEST_0615
   :status: open
   :verifies: REQ_0725

   Fixture EDS carrying every quirk listed in :need:`REQ_0725`:
   leading UTF-8 BOM, CRLF line endings, trailing whitespace on a
   value, ``; comment`` after a value, redundant whitespace around
   ``=``, ``LineFeed=0`` exporter key. The parse returns
   ``Ok(EdsFile)``; ``EdsFile::warnings`` lists one entry per
   triggered quirk with the expected ``EdsWarning::kind``.

----

Codegen / IR tests
------------------

Per-crate, snapshot-based. Live under
``crates/canopen-eds-codegen/tests/``.

.. test:: Name sanitisation handles EDS naming edge cases
   :id: TEST_0620
   :status: open
   :verifies: REQ_0731

   Parameterised test asserting the sanitisation map for a fixed
   table of EDS ``ProductName`` values → Rust idents: ``EPOS4 50/5``
   → ``EPOS4_50_5``, ``Module-V2`` → ``Module_V2``,
   ``1234 Drive`` → ``_1234_Drive``, empty / pure punctuation →
   error.

.. test:: Revision collision produces distinct idents
   :id: TEST_0621
   :status: open
   :verifies: REQ_0732

   Synthetic input set with two EDS files sharing ``ProductName``
   but differing in ``RevisionNumber`` (``0x01400000`` and
   ``0x01410000``). The generated module shall contain two
   distinct device structs with deterministic suffixes (e.g.
   ``EPOS4_REV0140`` and ``EPOS4_REV0141``). Reordering the input
   file list shall produce the same idents (string-compare of
   generated source).

.. test:: PDO entry dedup collapses structurally identical layouts
   :id: TEST_0622
   :status: open
   :verifies: REQ_0733

   Two synthetic devices whose RPDO entries have identical
   ``(bit_len, data_type)`` tuple lists but different field names
   produce a single shared PDO entry struct in the generated
   module. Asserted by counting distinct PDO-entry struct
   definitions in the ``TokenStream`` (expect 1, not 2).

.. test:: TokenStream emission, not string formatting
   :id: TEST_0623
   :status: open
   :verifies: REQ_0730, REQ_0734

   White-box test inside ``canopen-eds-codegen`` asserts that
   ``emit_device`` and ``emit_module_root`` return ``TokenStream``
   values directly (compile-time check via return type). A
   complementary lint forbids ``format!`` / ``write!`` /
   ``writeln!`` invocations within the codegen crate's source.

.. test:: One EDS file equals one device
   :id: TEST_0624
   :status: open
   :verifies: REQ_0735

   For an N-file fixture set, the call
   ``generate(&files, &SonicBackend::default())`` produces a
   module with exactly N distinct device-struct definitions and
   one shared registry table.

----

sonic backend snapshot tests
----------------------------

Live under ``crates/canopen-eds-codegen-sonic/tests/``.

.. test:: EPOS4 backend output snapshot
   :id: TEST_0630
   :status: open
   :verifies: REQ_0741, REQ_0742, REQ_0743, REQ_0744

   Run parse → codegen → backend → prettyplease on the canonical
   ``EPOS4`` EDS fixture. Compare the formatted output against a
   committed ``snapshots/epos4.rs`` golden file using
   ``insta::assert_snapshot!``. Reviewer regenerates the golden
   when intentional changes land; CI fails on unintentional
   churn.

.. test:: Generated registry covers every emitted device
   :id: TEST_0631
   :status: open
   :verifies: REQ_0745

   For an input set with N EDS files, the generated module's
   ``registry!()`` expansion contains exactly N entries mapping
   ``Identity → factory closure``. White-box test parses the
   generated output and counts entries.

.. test:: Generated module compiles under no_std + alloc
   :id: TEST_0632
   :status: open
   :verifies: REQ_0748

   A test crate at
   ``crates/canopen-eds-codegen-sonic/tests/no_std_consumer/``
   has ``#![no_std]`` and ``extern crate alloc;``, ``include!``s
   the generated module from a fixed input set, and compiles
   successfully. Catches any accidental ``std::``-qualified path
   in the backend's emit code.

.. test:: Backend is the sole canopen-eds-rt consumer in the toolchain
   :id: TEST_0633
   :status: open
   :verifies: REQ_0740

   CI shell check: ``cargo tree`` invocations for ``canopen-eds``,
   ``canopen-eds-codegen``, ``canopen-eds-build``,
   ``canopen-eds-cli``, and ``canopen-eds-verify`` must none of
   them list ``canopen-eds-rt`` in the dependency graph.
   ``canopen-eds-codegen-sonic`` is the only crate where
   ``canopen-eds-rt`` is allowed.

.. test:: Object-dictionary emission gated by feature flag
   :id: TEST_0634
   :status: open
   :verifies: REQ_0747

   Build the ``no_std_consumer`` test crate twice: once without
   features (the generated module exposes no ``OD`` symbol) and
   once with ``--features object-dictionary`` (the OD ``static``
   exists with the expected entry count for the input set).
   Compares the two binaries' rodata sections — the no-feature
   build is smaller by an amount approximating the OD table size.

.. test:: Dummy entries skipped in PDO struct fields
   :id: TEST_0635
   :status: open
   :verifies: REQ_0744

   Fixture EDS declares a PDO mapping containing one real
   ``0x6040:00`` entry, one ``Dummy32`` entry, and one real
   ``0x607A:00`` entry. The generated PDO payload struct
   contains exactly two fields (the two real entries). The
   generated ``encode`` body writes zeros in the bit range
   covered by the dummy; the generated ``decode`` body skips it.

.. test:: Bring-up SDO writes emitted from EDS
   :id: TEST_0636
   :status: open
   :verifies: REQ_0746

   Snapshot test on the generated ``impl CanOpenConfigurable``
   body for the EPOS4 fixture asserts the expected sequence of
   ``sdo.write(index, sub, value)`` calls: PDO comm params first,
   then PDO mapping params, then optional NMT start. Sequence and
   target indices match the EDS values exactly.

----

Runtime trait surface tests
---------------------------

Live under ``crates/canopen-eds-rt/tests/``.

.. test:: CanOpenDevice trait shape compiles for a hand-written device
   :id: TEST_0640
   :status: open
   :verifies: REQ_0750, REQ_0754, REQ_0755

   Hand-written test impl of ``CanOpenDevice`` for a minimal
   ``MockDevice`` validates the trait surface compiles end-to-end.
   Asserts ``IDENTITY`` is reachable, ``node_id`` / ``set_node_id``
   round-trip, ``nmt_state`` / ``set_nmt_state`` round-trip,
   ``on_rpdo`` is callable on ``&mut self`` with ``idx: u8`` in
   ``0..=3``, ``drain_tpdos`` is callable with a ``TpdoSink``, and
   each error path returns the expected ``CanOpenError`` variant
   (``PdoLenMismatch``, ``NmtStateViolation``, ``AbortCode``,
   ``TransportFailed``).

.. test:: CanOpenConfigurable async trait shape compiles
   :id: TEST_0641
   :status: open
   :verifies: REQ_0751

   Compile-only test: a mock device implements
   ``CanOpenConfigurable`` and an ``async fn configure`` body
   driving a mock ``SdoClient``. The test passes if compilation
   succeeds; catches any trait-method-async surface drift.

.. test:: canopen-eds-rt is the trait home, not sonic-internal
   :id: TEST_0642
   :status: open
   :verifies: REQ_0752

   CI shell check: ``rg "trait CanOpenDevice"`` across the
   workspace matches exactly one source location, inside
   ``crates/canopen-eds-rt/src/``. Same check for
   ``trait CanOpenConfigurable``.

.. test:: PdoOut payload uses heapless::Vec<u8, 8>
   :id: TEST_0643
   :status: open
   :verifies: REQ_0753

   Compile-time check: ``static_assertions::assert_type_eq_all!``
   on ``PdoOut::payload``'s type vs ``heapless::Vec<u8, 8>``. A
   change to the buffer type (e.g. lifting to a const generic for
   CAN-FD) requires this test to be updated deliberately rather
   than passing accidentally.

.. test:: RPDO rejected outside Operational state
   :id: TEST_0644
   :status: open
   :verifies: REQ_0756

   Integration test against a generated device fixture: set
   ``nmt_state(NmtState::PreOperational)``, call ``on_rpdo`` with
   a valid frame, assert the returned error is
   ``CanOpenError::NmtStateViolation``. Switch to
   ``NmtState::Operational`` and confirm the same frame succeeds.

----

Build helper tests
------------------

Live under ``crates/canopen-eds-build/tests/``.

.. test:: Builder writes a parseable Rust file to OUT_DIR
   :id: TEST_0650
   :status: open
   :verifies: REQ_0760, REQ_0761

   Test crate driven by a fixture EDS set runs
   ``Builder::new().glob(...).backend(default).out_file(
   "devices.rs").build()`` in a ``tempfile``-backed ``OUT_DIR``.
   Asserts the file exists, is non-empty, and parses with
   ``syn::parse_file`` (catches malformed token streams).

.. test:: cargo rerun-if-changed emitted per EDS input
   :id: TEST_0651
   :status: open
   :verifies: REQ_0762

   Capture ``Builder::build()``'s stdout. For an N-file glob,
   assert exactly N + 1 ``cargo:rerun-if-changed=`` lines are
   present (one per EDS file + one for the build script itself).

.. test:: Output passes prettyplease formatting
   :id: TEST_0652
   :status: open
   :verifies: REQ_0763

   The generated ``devices.rs`` file is line-wrapped, and
   re-running ``prettyplease::unparse`` on the file produces a
   byte-identical output (idempotent formatter pass).

.. test:: Parser warnings surface as cargo:warning lines
   :id: TEST_0653
   :status: open
   :verifies: REQ_0764

   Drive the builder against a fixture set including the
   liberal-quirk EDS from :need:`TEST_0615`. Capture stdout, assert
   that one ``cargo:warning=`` line appears per quirk warning
   raised by the parser, and that the build still exits ``Ok``.

----

CLI tests
---------

Live under ``crates/canopen-eds-cli/tests/``.

.. test:: cargo eds expand emits a single device's code
   :id: TEST_0660
   :status: open
   :verifies: REQ_0770

   Spawn the CLI as ``cargo eds expand --device EPOS4 --glob
   <fixtures>/*.eds``, capture stdout, assert the output is
   non-empty, parses as Rust, and contains exactly one ``pub
   struct EPOS4_REV0140`` (or equivalent rev-suffixed) definition.

.. test:: cargo eds list enumerates devices
   :id: TEST_0661
   :status: open
   :verifies: REQ_0771

   Run ``cargo eds list --glob <fixtures>/*.eds`` over a
   3-device fixture set, assert stdout contains 3 lines each
   matching the ``<ident>\t<vendor_id>\t<product_code>\t<revision>``
   format.

.. test:: CLI output matches build helper output byte-for-byte
   :id: TEST_0662
   :status: open
   :verifies: REQ_0772

   For one fixed device, capture the CLI's ``expand`` output and
   the build helper's per-device slice of ``$OUT_DIR/devices.rs``.
   Assert the two byte-strings are identical (catches divergent
   code paths between CLI and build).

----

Verifier tests
--------------

Live under ``crates/canopen-eds-verify/tests/``.

.. test:: Verifier passes on matching EDS + dump pair
   :id: TEST_0670
   :status: open
   :verifies: REQ_0780

   Use the captured pair from
   ``crates/canopen-eds-verify/tests/fixtures/EPOS4/`` (EDS file
   + matching ``epos4.dump.json``), call ``verify(eds, dump)``,
   assert ``Ok(VerifyReport { matched: true, .. })``.

.. test:: Verifier reports the differing field
   :id: TEST_0671
   :status: open
   :verifies: REQ_0781

   Synthetic mismatched pair: real EDS, mutated JSON dump
   altering the ``0x1018:02`` product_code field. The returned
   ``VerifyReport`` shall contain at least one ``Difference``
   entry whose ``field`` is exactly ``"Identity.product_code"``
   and whose ``eds`` / ``dump`` values match the originals.

.. test:: Verifier reuses canopen-eds parser
   :id: TEST_0672
   :status: open
   :verifies: REQ_0782

   White-box: ``cargo tree -p canopen-eds-verify`` lists
   ``canopen-eds`` as a direct dependency and does **not** list
   ``canopen-eds-codegen``, ``canopen-eds-rt``,
   ``sonic-connector-can``, or ``socketcan`` anywhere in the
   graph.

.. test:: Verifier exit codes follow the documented matrix
   :id: TEST_0673
   :status: open
   :verifies: REQ_0783

   Spawn the verifier binary three times: matching pair (expect
   exit 0), mismatched pair (expect 1), unreadable JSON path
   (expect 2). Asserted via ``Command::status().code()``.

.. test:: Verifier rejects unknown schema version
   :id: TEST_0674
   :status: open
   :verifies: REQ_0784

   Fixture JSON dump with ``"schema": "sonic.canopen.sdo-dump.v2"``.
   The verifier shall reject the input with a parse error (exit
   code ``2``) before any field comparison runs.

----

Cross-cutting reproducibility tests
-----------------------------------

Verify the build-time determinism quality goal
(:need:`QG_0014`).

.. test:: Repeated codegen runs produce byte-identical output
   :id: TEST_0680
   :status: open
   :verifies: QG_0014, REQ_0763

   Run ``Builder::build()`` twice on the same input set in
   freshly-prepared ``OUT_DIR`` directories. Compare the two
   ``devices.rs`` files with ``sha256``. Assert identical.

.. test:: Input-file ordering does not affect output
   :id: TEST_0681
   :status: open
   :verifies: QG_0014, REQ_0732, REQ_0733

   Same input set, glob returns files in two different orders
   (force the order via explicit ``Builder::file(path)`` calls).
   The two ``devices.rs`` outputs are byte-identical (catches
   HashMap-iteration-order nondeterminism in dedup or collision-
   handling).

.. test:: Layering integrity check (Cargo.toml audit)
   :id: TEST_0682
   :status: open
   :verifies: QG_0015, REQ_0721, REQ_0740

   CI shell check that walks each toolchain crate's
   ``Cargo.toml`` and asserts the allowed-dependency matrix:

   * ``fieldbus-od-core``: no ``ethercrab``, no ``socketcan``, no
     ``sonic-connector-*``, no ``canopen-eds-rt``.
   * ``canopen-eds``: no ``canopen-eds-codegen``,
     no ``canopen-eds-rt``, no ``socketcan``, no ``ethercrab``.
   * ``canopen-eds-codegen``: no ``canopen-eds-rt``, no transport
     crates.
   * ``canopen-eds-build``, ``canopen-eds-cli``: no
     ``canopen-eds-rt``.
   * ``canopen-eds-verify``: no ``canopen-eds-codegen``, no
     ``canopen-eds-rt``, no transport crates.

   Implemented with ``cargo metadata`` + ``jq``; runs in the
   workspace CI job.

----

Cross-cutting traceability
--------------------------

.. needtable::
   :types: test
   :filter: id >= "TEST_0600" and id <= "TEST_0699"
   :columns: id, title, status, verifies
   :show_filters:
