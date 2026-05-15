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
# Regex note: `cargo tree` indents children with multi-byte UTF-8
# tree-drawing characters (├ └ │ ─). A byte-class like `[├└│ ]`
# does NOT match those bytes reliably across locales. We use
# `^[^a-zA-Z0-9]*zenoh v[0-9]` instead — anchor at start, allow any
# non-alphanumeric prefix (spaces + UTF-8 high-bit bytes), then
# require the literal lowercase `zenoh v<digit>`. This correctly
# rejects the workspace-crate self-line `sonic-connector-zenoh
# v0.1.0` (begins with alphanumeric `s`).

set -Eeuo pipefail
shopt -s inherit_errexit 2>/dev/null || true

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
cd -- "${SCRIPT_DIR}/.."

readonly PKG="sonic-connector-zenoh"
readonly FEATURE="zenoh-integration"
readonly ZENOH_RE='^[^a-zA-Z0-9]*zenoh v[0-9]'
readonly ZENOH_V1_RE='^[^a-zA-Z0-9]*zenoh v1\.'

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
