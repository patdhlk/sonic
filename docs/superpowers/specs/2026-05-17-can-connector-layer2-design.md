# Spec: FEAT_0046 — CAN connector, layer-2 (real socketcan)

Generated 2026-05-17. Continuation of
`2026-05-17-can-connector-layer1-design.md`; covers the deferred
"real socketcan integration" scope from that doc.

Source requirements (already authored — this doc adds no new reqs):
**FEAT_0046**, **REQ_0502** (Linux is the supported host OS for real
I/O), **REQ_0503** (socketcan-integration feature gates the real
socketcan dep), **REQ_0631** (error frames consumed internally via
CAN_ERR_FLAG), **REQ_0633** (bus-off triggers reconnect).

## Layer-2 scope

| Area | Layer-1 | Layer-2 |
|------|---------|---------|
| `socketcan-integration` feature | declared empty | wires `dep:socketcan` under a Linux-only target table |
| `RealCanInterface` | absent | new module `src/real.rs`, `cfg(target_os = "linux")` |
| TEST_0511 (dep gating) | `:status: open` (no impl) | shell + cargo-tree check landed, `:status: implemented` |
| TEST_0512 (vcan smoke) | `:status: draft` | `tests/vcan_smoke.rs`, `:status: implemented`, `#[ignore]` so it only runs in CI with the `vcan` kernel module |
| `.github/workflows/ci-can.yml` | absent | new workflow: build / clippy / test on Linux, with and without `socketcan-integration`; vcan smoke job |
| `IMPL_0080` description | "(layer-1)" | extend bullets to include `RealCanInterface` surface |

## API surface (from socketcan 3.5.0 source + context7)

- `socketcan::tokio::CanFdSocket::open(ifname) -> IoResult<Self>` — open one FD-aware kernel socket (handles both classical and FD frames per Linux `CAN_RAW_FD_FRAMES` semantics).
- `SocketOptions::set_filters(&[CanFilter])` — apply per-iface filter union (REQ_0522 mechanism).
- `SocketOptions::set_error_filter(mask: u32)` — enable error-frame reporting (REQ_0631); call with `socketcan::CAN_ERR_MASK` to accept all.
- `CanFdSocket::read_frame() -> IoResult<CanAnyFrame>` — yields `CanAnyFrame::Normal(CanFrame::{Data|Remote|Error})` or `CanAnyFrame::Fd(CanFdFrame)`.
- `CanDataFrame::new(id: impl Into<Id>, data: &[u8])` — build classical frame (via `EmbeddedFrame`).
- `CanFdFrame::with_flags(id, data, FdFlags)` — build FD frame with BRS / ESI.
- `CanErrorFrame::into_error() -> CanError` — classify error-frame bits into a typed enum (BusOff, ControllerProblem(Receive/Transmit Error{Warning|Passive}), LostArbitration, …).

## Identifier conversion

| Sonic `CanId` | embedded_can `Id` |
|---|---|
| `{value: u, extended: false}` | `Id::Standard(StandardId::new(u as u16).unwrap())` |
| `{value: u, extended: true}` | `Id::Extended(ExtendedId::new(u).unwrap())` |

Unwraps are safe because layer-1's `CanId::standard / ::extended` constructors already bounds-check the values.

## Error-class mapping

`CanError` from socketcan → my `CanErrorKind`:

| socketcan `CanError` variant | sonic `CanErrorKind` |
|---|---|
| `BusOff` | `BusOff` |
| `ControllerProblem(Receive\|Transmit ErrorPassive)` | `Passive` |
| `ControllerProblem(Receive\|Transmit ErrorWarning)` | `Warning` |
| `LostArbitration(_)` | `ArbitrationLost` |
| anything else | `Other` |

## Plan table

| # | Task | Skill | Target | Detail | File | Required |
|---|------|-------|--------|--------|------|----------|
| 1 | Wire the optional socketcan dep | (manual edit) | `crates/sonic-connector-can/Cargo.toml` | Change `socketcan-integration = []` → `["dep:socketcan"]`. Add socketcan dep under `[target.'cfg(target_os = "linux")'.dependencies]` with `default-features = false, features = ["tokio"]`. | `crates/sonic-connector-can/Cargo.toml` | recommended |
| 2 | Implement RealCanInterface | (manual write) | `crates/sonic-connector-can/src/real.rs` | Linux-only `RealCanInterface` wrapping `socketcan::tokio::CanFdSocket`. Implement `CanInterfaceLike`: open, recv (CanAnyFrame → CanFrame), send_classical / send_fd, apply_filter (kernel CanFilter conversion), reopen (drop + reopen socket), state. Enable `set_error_filter(CAN_ERR_MASK)` on open (REQ_0631). | `crates/sonic-connector-can/src/real.rs` | recommended |
| 3 | Wire module + re-export | (manual edit) | `crates/sonic-connector-can/src/lib.rs` | `#[cfg(all(feature = "socketcan-integration", target_os = "linux"))] pub mod real;` + re-export `RealCanInterface`. | `crates/sonic-connector-can/src/lib.rs` | recommended |
| 4 | TEST_0511 dep-gating script | (manual write) | `crates/sonic-connector-can/scripts/check_dep_gating.sh` | Shell script asserts `cargo tree` shows `socketcan` only when `--features socketcan-integration` is passed. Mirrors `scripts/check_dep_gating.sh` from the zenoh crate. | `crates/sonic-connector-can/scripts/check_dep_gating.sh` | recommended |
| 5 | TEST_0512 vcan smoke test | (manual write) | `crates/sonic-connector-can/tests/vcan_smoke.rs` | `cfg(all(feature = "socketcan-integration", target_os = "linux"))`. `#[tokio::test] #[ignore]` so it only runs when CI passes `--include-ignored`. Opens `vcan0`, round-trips one classical frame, asserts. | `crates/sonic-connector-can/tests/vcan_smoke.rs` | recommended |
| 6 | Add ci-can.yml | (manual write) | `.github/workflows/ci-can.yml` | Linux runner. Jobs: `build-default` (no features), `build-integration` (`--features socketcan-integration`), `clippy-integration` (clippy with the feature), `dep-gating` (run the script), `vcan-smoke` (modprobe vcan + cargo test --include-ignored). | `.github/workflows/ci-can.yml` | recommended |
| 7 | Update spec statuses | (manual edit) | `spec/verification/connector.rst`, `spec/architecture/connector.rst` | Flip TEST_0511 to `:status: implemented` with `:links:` to scripts/CI; flip TEST_0512 to `:status: implemented` with `:links:` to the test file + CI job. Extend IMPL_0080's surface bullet list to include `RealCanInterface`. | spec/ | recommended |
| 8 | Validate + commit + push | (manual run) | `pre-commit run` + `git push` | All hooks pass; CI green on Linux runners. | — | recommended |

## Out of scope

- `RealCanInterface` for non-Linux hosts (the dep is Linux-only — REQ_0502).
- Per-iface socket-mode configuration (classical-only vs FD-aware). Layer-2 always opens FD-aware sockets; classical channels work transparently because Linux's `CAN_RAW_FD_FRAMES` accepts both.
- DBC / ISO-TP / J1939 / CAN-XL — explicit anti-goals (REQ_0640–0642).
- Send-loopback control (`CAN_RAW_RECV_OWN_MSGS`) — left at kernel default (off).
