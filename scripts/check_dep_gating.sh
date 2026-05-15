#!/usr/bin/env bash
# TEST_0310 — verify that the `zenoh-integration` cargo feature
# gates the real `zenoh` dependency. Verifies REQ_0444 + REQ_0445.
#
# Exits non-zero if:
#   - the default build pulls `zenoh`, OR
#   - the feature build does NOT pull `zenoh v1.x`, OR
#   - `sonic-connector-zenoh` fails to type-check in either
#     configuration (MockZenohSession unreachable).
#
# Run from the workspace root (or any directory — the script
# `cd`s to its own parent's parent).
#
# Regex note: matching against `cargo tree` output cannot rely on
# the tree-drawing prefix — `cargo tree` may emit either multi-byte
# UTF-8 box-drawing chars (├ └ │ ─) or plain whitespace depending on
# terminal-detection heuristics (CI is non-tty). Instead of anchoring
# on the prefix, we require the literal token ` zenoh v<digit>` with
# a leading space — that space distinguishes the real `zenoh v1.x`
# dep lines from the workspace self-line `sonic-connector-zenoh
# v0.1.0` (no space between `sonic-connector-` and `zenoh`).

set -Eeuo pipefail
shopt -s inherit_errexit 2>/dev/null || true

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
cd -- "${SCRIPT_DIR}/.."

readonly PKG="sonic-connector-zenoh"
readonly FEATURE="zenoh-integration"
readonly ZENOH_RE=' zenoh v[0-9]'
readonly ZENOH_V1_RE=' zenoh v1\.[0-9]'

printf '==> TEST_0310: default build must not link zenoh\n'
default_tree="$(cargo tree -p "${PKG}" --no-default-features --edges normal 2>/dev/null)"
if printf '%s\n' "${default_tree}" | grep -E "${ZENOH_RE}" >/dev/null; then
    printf 'FAIL: zenoh dep present in default build\n' >&2
    printf '%s\n' "${default_tree}" | grep -E 'zenoh v[0-9]' >&2
    exit 1
fi
printf '  ok: default build does not pull zenoh\n'

printf '==> TEST_0310: feature build must link zenoh v1.x\n'
feature_tree="$(cargo tree -p "${PKG}" --features "${FEATURE}" --edges normal 2>/dev/null)"
if ! printf '%s\n' "${feature_tree}" | grep -E "${ZENOH_V1_RE}" >/dev/null; then
    printf 'FAIL: zenoh v1.x missing from feature build\n' >&2
    printf '%s\n' "${feature_tree}" >&2
    exit 1
fi
printf '  ok: feature build pulls zenoh v1.x\n'

printf '==> TEST_0310: MockZenohSession reachable in default build\n'
cargo check -p "${PKG}" --no-default-features --tests

printf '==> TEST_0310: MockZenohSession reachable in feature build\n'
cargo check -p "${PKG}" --features "${FEATURE}" --tests

printf 'PASS: TEST_0310 — cargo-feature dep gating verified\n'
