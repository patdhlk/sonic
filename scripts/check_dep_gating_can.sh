#!/usr/bin/env bash
# TEST_0511 — verify that the `socketcan-integration` cargo feature
# gates the real `socketcan` dependency. Verifies REQ_0503, REQ_0504.
#
# Exits non-zero if:
#   - the default build pulls `socketcan`, OR
#   - the feature build (on Linux) does NOT pull `socketcan v3.x`, OR
#   - `sonic-connector-can` fails to type-check in either
#     configuration (MockCanInterface unreachable).
#
# On non-Linux hosts the feature build legitimately omits the dep
# (target-cfg-gated under `cfg(target_os = "linux")`), so the
# feature-build assertion is skipped with a notice.
#
# Same regex / tree-prefix rationale as
# scripts/check_dep_gating.sh — match on a leading space so the
# crate's self-line (`sonic-connector-can v0.1.0`) is not
# misclassified.

set -Eeuo pipefail
shopt -s inherit_errexit 2>/dev/null || true

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
cd -- "${SCRIPT_DIR}/.."

readonly PKG="sonic-connector-can"
readonly FEATURE="socketcan-integration"
readonly SOCKETCAN_RE=' socketcan v[0-9]'
readonly SOCKETCAN_V3_RE=' socketcan v3\.[0-9]'

printf '==> TEST_0511: default build must not link socketcan\n'
default_tree="$(cargo tree -p "${PKG}" --no-default-features --edges normal 2>/dev/null)"
if printf '%s\n' "${default_tree}" | grep -E "${SOCKETCAN_RE}" >/dev/null; then
    printf 'FAIL: socketcan dep present in default build\n' >&2
    printf '%s\n' "${default_tree}" | grep -E 'socketcan v[0-9]' >&2
    exit 1
fi
printf '  ok: default build does not pull socketcan\n'

uname_s="$(uname -s)"
if [[ "${uname_s}" == "Linux" ]]; then
    printf '==> TEST_0511: feature build must link socketcan v3.x (Linux)\n'
    feature_tree="$(cargo tree -p "${PKG}" --features "${FEATURE}" --edges normal 2>/dev/null)"
    if ! printf '%s\n' "${feature_tree}" | grep -E "${SOCKETCAN_V3_RE}" >/dev/null; then
        printf 'FAIL: socketcan v3.x missing from feature build\n' >&2
        printf '%s\n' "${feature_tree}" >&2
        exit 1
    fi
    printf '  ok: feature build pulls socketcan v3.x\n'
else
    printf '==> TEST_0511: feature build on %s — socketcan dep is Linux-only, skipping\n' "${uname_s}"
fi

printf '==> TEST_0511: MockCanInterface reachable in default build\n'
cargo check -p "${PKG}" --no-default-features --tests

printf '==> TEST_0511: MockCanInterface reachable in feature build\n'
cargo check -p "${PKG}" --features "${FEATURE}" --tests

printf 'PASS: TEST_0511 — cargo-feature dep gating verified\n'
