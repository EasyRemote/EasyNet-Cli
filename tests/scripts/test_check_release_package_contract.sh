#!/usr/bin/env bash
#
# Contract tests for scripts/check-release-package-contract.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/scripts/check-release-package-contract.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p "$sandbox/scripts" "$sandbox/tests" "$sandbox/include" "$sandbox/docs/spec"
    cp "$REPO_ROOT/scripts/build-release-tarball.sh" "$sandbox/scripts/build-release-tarball.sh"
    cp "$REPO_ROOT/scripts/e2e-release-install.sh" "$sandbox/scripts/e2e-release-install.sh"
    cp "$REPO_ROOT/install.sh" "$sandbox/install.sh"
    cp "$REPO_ROOT/include/easynet_cli.h" "$sandbox/include/easynet_cli.h"
    cp "$REPO_ROOT/docs/spec/ffi-abi-v3.md" "$sandbox/docs/spec/ffi-abi-v3.md"
    echo "$sandbox"
}

run_check() {
    local sandbox="$1"
    ( cd "$sandbox" && CHECK_RELEASE_PACKAGE_CONTRACT_ROOT="$sandbox" bash "$SCRIPT" )
}

SB="$(make_sandbox)"
run_check "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "happy: release package contract should pass"; }
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/ --bin easynet-keyring//' "$SB/scripts/build-release-tarball.sh"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing keyring build should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's#include/easynet_cli\.h#include/missing_header.h#g' \
    "$SB/scripts/build-release-tarball.sh"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing ABI header in tarball script should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's#INCLUDE_DIR="/usr/local/include/easynet"##' "$SB/install.sh"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "installer missing include dir should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/#define EASYNET_ABI_VERSION 3u/#define EASYNET_ABI_VERSION 2u/' \
    "$SB/scripts/e2e-release-install.sh"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "e2e install missing ABI v3 assertion should exit 1 (got $rc)"

echo "test_check_release_package_contract.sh: all cases passed"
