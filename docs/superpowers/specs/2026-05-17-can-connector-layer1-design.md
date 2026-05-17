# Spec: FEAT_0046 — CAN (SocketCAN) reference connector, layer-1 implementation

Generated from sphinx-needs on 2026-05-17 via `pharaoh:spec`.
Source requirements: FEAT_0046, FEAT_0047, FEAT_0048, FEAT_0049.

Scope: deliver a functionally complete CAN connector against the
`MockCanInterface` (layer-1). Real `socketcan::tokio::*` integration
(layer-2) lands in a follow-on spec — the `socketcan-integration` cargo
feature exists but is unimplemented in this turn.

## Requirements (source of truth)

### FEAT_0046: CAN (SocketCAN) reference connector

**Status:** open · **Satisfies:** FEAT_0030

A fourth concrete connector instantiating the framework's contracts:
`socketcan`-backed CAN plugin and gateway exchanging classical CAN and
CAN-FD frames on one or more Linux SocketCAN network interfaces, with
internal error-frame-driven health, `ReconnectPolicy`-driven bus-off
recovery, and a non-Linux `MockCanInterface` for layer-1 tests. The
gateway owns N `socketcan::CanSocket` / `CanFdSocket` instances — one
per registered interface — and runs the RX/TX loops on a tokio sidecar
contained inside `sonic-connector-can`. Linux is the only supported
host OS for real I/O; the in-process mock interface is portable.

### FEAT_0047: CAN frame transport (classical + FD)

**Status:** open · **Satisfies:** FEAT_0046

The on-wire form of CAN traffic crossing the plugin↔gateway boundary.
`CanRouting` declares per-channel CAN ID, mask, and frame kind
(Classical or FD); the iceoryx2 service payload carries the raw CAN
data bytes (codec-encoded plugin-side per REQ_0211), with the gateway
acting as a byte-only mover (symmetric with REQ_0327 and REQ_0408).

### FEAT_0048: Multi-interface gateway and per-channel filtering

**Status:** open · **Satisfies:** FEAT_0046

The gateway-side multiplexer: one gateway instance can own multiple
Linux CAN interfaces (broader than REQ_0312's single-MainDevice
EtherCAT posture). Per-channel CAN ID and mask are compiled into one
`CAN_RAW_FILTER` `setsockopt` per interface, recomputed when channels
are added or removed.

### FEAT_0049: Bus health, error frames, and reconnect

**Status:** open · **Satisfies:** FEAT_0046

The CAN-specific health surface: per-interface state aggregated into
the connector's single externally-visible `ConnectorHealth`,
error-frame consumption driving transitions internally, and
`ReconnectPolicy`-driven socket reopen on bus-off. Health-event
semantics inherit from FEAT_0034.

## Existing coverage

| Need | Type | Title | Status | Links |
|------|------|-------|--------|-------|
| BB_0070 | building-block | sonic-connector-can crate | open | implements REQ_0600, REQ_0602, REQ_0603, REQ_0604, REQ_0605 |
| BB_0071 | building-block | CanConnector (plugin) | open | implements REQ_0600, REQ_0601, REQ_0612, REQ_0615, REQ_0621 |
| BB_0072 | building-block | CanGateway | open | implements REQ_0613, REQ_0614, REQ_0620, REQ_0624, REQ_0625, REQ_0630, REQ_0631 |
| BB_0073 | building-block | Tokio bridge for CAN | open | implements REQ_0605, REQ_0606, REQ_0607, REQ_0608 |
| BB_0074 | building-block | Per-iface filter compiler | open | implements REQ_0622, REQ_0623, REQ_0624 |
| BB_0075 | building-block | MockCanInterface | open | implements REQ_0604 |
| ARCH_0060 | architecture | CAN frame send path | open | refines REQ_0613, REQ_0621, BB_0072, BB_0073 |
| ARCH_0061 | architecture | CAN receive path with multi-iface demux | open | refines REQ_0614, REQ_0620, REQ_0622, REQ_0624, BB_0072, BB_0074 |
| ARCH_0062 | architecture | CAN bus health and bus-off recovery | open | refines REQ_0630, REQ_0632, REQ_0633, REQ_0634, REQ_0635, BB_0072 |
| IMPL_0080 | impl | sonic-connector-can crate (planned) | draft | implements BB_0070; refines all 28 positive reqs |
| TEST_0500–0511, TEST_0513–0514 | test | layer-1 + feature-gating coverage | open | verifies 26 of 28 reqs |
| TEST_0512 | test | Linux raw-socket smoke against vcan0 | draft | layer-2, deferred |

Coverage hit rate: **28/28 reqs have BB coverage**, **26/28 have TEST coverage** (gaps below).

## Gaps

- [ ] **REQ_0612** (Channel payload sizing keyed on frame kind) — lacks an explicit `:verifies:` link. Implicitly covered by TEST_0502 (classical 1–8 bytes) and TEST_0503 (FD DLC steps to 64). Fix: extend `TEST_0502`'s `:verifies:` list to include `REQ_0612`.
- [ ] **REQ_0631** (Error frames consumed internally / `CAN_ERR_FLAG` enabled) — lacks an explicit `:verifies:` link. Implicitly covered by TEST_0507 (error-passive → Degraded) and TEST_0513 (anti-req — error frames not on plugin channel). Fix: extend `TEST_0513`'s `:verifies:` list to include `REQ_0631`.
- [ ] **Code does not exist** at `crates/sonic-connector-can/`. This spec drives its creation per IMPL_0080's planned surface and the EtherCAT template at `crates/sonic-connector-ethercat/`.

## Decisions

No formal design decisions required. The two link-extension gaps are
mechanical; the implementation strategy is fully constrained by the
existing spec (REQ_0600–0636 + BB_0070–0075 + IMPL_0080 surface) and
the EtherCAT template.

## Implementation scope

### Needs to create

| Type | Purpose | Links to | File |
|------|---------|----------|------|
| (rust crate) | layer-1 CAN connector implementation | implements IMPL_0080 / BB_0070 | `crates/sonic-connector-can/` (12 source modules + tests) |

### Needs to modify

| Need | Change | Reason |
|------|--------|--------|
| TEST_0502 | Add `REQ_0612` to `:verifies:` list | Existing test already exercises payload sizing per frame kind; link makes the trace explicit. |
| TEST_0513 | Add `REQ_0631` to `:verifies:` list | Existing anti-req regression-guard already exercises the error-frame consumption path; link makes the trace explicit. |
| `Cargo.toml` (workspace) | Add `crates/sonic-connector-can` to `members` | New workspace crate. |

## Implementation phasing (layer-1)

Module-by-module, mirroring `crates/sonic-connector-ethercat/src/`:

| # | Module | LOC est. | Mirrors | Realises |
|---|--------|---------|---------|----------|
| 1 | `Cargo.toml` + `lib.rs` | 60 | ethercat | crate scaffold |
| 2 | `routing.rs` | 130 | ethercat routing + extras | CanIface, CanId, CanFrameKind, CanFdFlags, CanRouting (REQ_0601, REQ_0615) |
| 3 | `options.rs` | 210 | ethercat options | CanConnectorOptions builder (REQ_0506, REQ_0520, REQ_0534) |
| 4 | `bridge.rs` | 160 | ethercat bridge (verbatim shape) | OutboundBridge / InboundBridge (REQ_0506–0608) |
| 5 | `health.rs` | 220 | ethercat health + per-iface aggregator | CanHealthMonitor with worst-of aggregation (REQ_0630, REQ_0635) |
| 6 | `registry.rs` | 200 | ethercat registry | Per-iface routing registry (REQ_0525) |
| 7 | `filter.rs` | 180 | (new — no ethercat equivalent) | BB_0074 — compile filter union, match incoming frames |
| 8 | `driver.rs` | 90 | ethercat driver | CanInterfaceLike trait + CanFrame enum |
| 9 | `mock.rs` | 280 | ethercat mock | MockCanInterface loopback + programmable error injection (BB_0075) |
| 10 | `gateway.rs` | 95 | ethercat gateway | Tokio runtime owner (REQ_0505) |
| 11 | `dispatcher.rs` | 320 | ethercat dispatcher | Per-iface RX/TX loops + classifier + bus-off → reconnect (ARCH_0061, ARCH_0062) |
| 12 | `connector.rs` | 380 | ethercat connector | Connector trait impl + create_writer/reader (REQ_0600, REQ_0524) |
| 13 | `tests/` | 600 | ethercat tests | TEST_0500–0510 against MockCanInterface |

Estimated total ~2925 LOC including tests. Real socketcan binding (`socketcan_driver.rs` ~250 LOC + integration test) deferred.

## Plan table

| # | Task | Skill | Target | Detail | File | Required |
|---|------|-------|--------|--------|------|----------|
| 1 | Add CAN crate to workspace | (manual edit) | `Cargo.toml` (workspace root) | Append `crates/sonic-connector-can` to `[workspace] members`. | `Cargo.toml` | recommended |
| 2 | Scaffold Cargo.toml + lib.rs | (manual write) | `crates/sonic-connector-can/Cargo.toml`, `src/lib.rs` | Mirror ethercat manifest; declare optional `socketcan` dep behind `socketcan-integration` feature; deny `unsafe_code`. | `crates/sonic-connector-can/` | recommended |
| 3 | Author types | (manual write) | `src/routing.rs`, `src/options.rs`, `src/driver.rs` | CanIface, CanId, CanFrameKind, CanFdFlags, CanRouting, CanFrame enum, CanInterfaceLike trait, CanConnectorOptions + builder. | `crates/sonic-connector-can/src/` | recommended |
| 4 | Author plumbing | (manual write) | `src/bridge.rs`, `src/health.rs`, `src/registry.rs`, `src/filter.rs` | Bridges (verbatim shape from ethercat), CanHealthMonitor with worst-of aggregator, per-iface registry, filter compiler + matcher. | `crates/sonic-connector-can/src/` | recommended |
| 5 | Author MockCanInterface | (manual write) | `src/mock.rs` | In-process loopback with programmable bus-state injection. | `crates/sonic-connector-can/src/mock.rs` | recommended |
| 6 | Author gateway + dispatcher | (manual write) | `src/gateway.rs`, `src/dispatcher.rs` | Per-iface RX/TX loops, error classifier, bus-off → reconnect. | `crates/sonic-connector-can/src/` | recommended |
| 7 | Author connector + register_with | (manual write) | `src/connector.rs` | Connector trait impl, create_writer/create_reader with iface validation, register_with spawns dispatcher. | `crates/sonic-connector-can/src/connector.rs` | recommended |
| 8 | Layer-1 integration tests | (manual write) | `tests/*.rs` | TEST_0500 (trait surface), TEST_0501 (routing round-trip), TEST_0502 / TEST_0503 (classical + FD round-trip), TEST_0504 (filter union), TEST_0505 (multi-iface demux), TEST_0506 (bus-off → reconnect), TEST_0507 (error-passive → Degraded), TEST_0508 (tokio sidecar containment), TEST_0509 / TEST_0510 (bridge saturation), TEST_0513 (anti-req error frames hidden), TEST_0514 (registry alloc-free iter). | `crates/sonic-connector-can/tests/` | recommended |
| 9 | Patch verifies links | (manual edit) | `spec/verification/connector.rst` | Add `REQ_0612` to TEST_0502, add `REQ_0631` to TEST_0513. | `spec/verification/connector.rst` | recommended |
| 10 | Update IMPL_0080 status | (manual edit) | `spec/architecture/connector.rst` | Flip IMPL_0080 from `draft` to `open` once layer-1 surface matches the planned bullets. | `spec/architecture/connector.rst` | recommended |
| 11 | Verify hooks + CI | (manual run) | `pre-commit run --all-files` + `pre-commit run --hook-stage pre-push --all-files` | All hooks pass; cargo clippy + cargo doc clean; sphinx -W clean. | — | recommended |
| 12 | Commit + push | (manual git) | `main` | Single commit per phase (or one combined); push and watch CI. | — | recommended |

Plan tasks 2–8 are bounded by the EtherCAT template — the shape is
known, the risk is execution, not design. Task 11 catches any drift
before it reaches CI.

## Deferred (out of scope for this spec)

- **Real socketcan integration** — `src/socketcan_driver.rs` (~250 LOC), `tests/vcan0_smoke.rs` (TEST_0512), feature-gated `cargo tree` dep-gating test (TEST_0511 wiring), `.github/workflows/ci-can.yml`. Authored in a follow-on spec once layer-1 lands.
- **Pharaoh decision artefacts (DEC / ADR)** — no formal decisions surfaced during this spec generation. If layer-2 reveals genuine forks (e.g. how to wire `CAN_RAW_FD_FRAMES` ↔ socket type), record them at that point.
