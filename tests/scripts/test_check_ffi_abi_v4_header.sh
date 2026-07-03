#!/usr/bin/env bash
#
# Contract tests for tools/scripts/check-ffi-abi-v4-header.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-ffi-abi-v4-header.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p "$sandbox/include" "$sandbox/src" "$sandbox/docs/spec"
    cp "$REPO_ROOT/include/easynet_cli.h" "$sandbox/include/easynet_cli.h"
    cp -R "$REPO_ROOT/src/ffi" "$sandbox/src/ffi"
    cp "$REPO_ROOT/docs/spec/ffi-abi-v4.md" "$sandbox/docs/spec/ffi-abi-v4.md"
    echo "$sandbox"
}

run_check() {
    local sandbox="$1"
    ( cd "$sandbox" && CHECK_FFI_ABI_V4_HEADER_ROOT="$sandbox" bash "$SCRIPT" )
}

SB="$(make_sandbox)"
run_check "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "happy: clean ABI header should pass"; }
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/#define EASYNET_ABI_VERSION 4u/#define EASYNET_ABI_VERSION 2u/' \
    "$SB/include/easynet_cli.h"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "ABI version drift should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/#define ERR_INVALID_ARG 11\n//' "$SB/include/easynet_cli.h"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing error code should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/pub const ERR_INVALID_ARG: i32 = 11;/pub const ERR_INVALID_ARG: i32 = 42;/' \
    "$SB/src/ffi/errors/mod.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "Rust error code drift should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/int32_t easynet_invocation_bidi_open/int32_t easynet_invocation_bidi_start/' \
    "$SB/include/easynet_cli.h"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "renamed bidi open symbol should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/int32_t easynet_invocation_prepare/int32_t easynet_invocation_prepare_missing/' \
    "$SB/include/easynet_cli.h"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "renamed prepare symbol should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/int32_t easynet_invocation_builder_prepare/int32_t easynet_invocation_builder_prepare_missing/' \
    "$SB/include/easynet_cli.h"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "renamed builder prepare symbol should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/int32_t easynet_invocation_handle_await/int32_t easynet_invocation_handle_wait_missing/' \
    "$SB/include/easynet_cli.h"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "renamed invocation handle await symbol should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/int32_t easynet_invocation_bidi_close_send/int32_t easynet_invocation_bidi_half_close_missing/' \
    "$SB/include/easynet_cli.h"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "renamed bidi close-send symbol should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/fn easynet_invocation_bidi_open/fn easynet_invocation_bidi_start/' \
    "$SB/src/ffi/invocation/mod.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "Rust-exported bidi symbol drift should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '\ntypedef uint64_t EasynetSubscriptionId;\nint32_t easynet_ability_invoke(void);\n' \
    >>"$SB/include/easynet_cli.h"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired ability+args ABI should exit 1 (got $rc)"

SB="$(make_sandbox)"
cat >"$SB/src/ffi/ability.rs" <<'RS'
#[no_mangle]
pub unsafe extern "C" fn easynet_ability_invoke() -> i32 { 0 }
RS
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired Rust ability module should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '\ntypedef uint32_t EasynetInitMode;\n#define EASYNET_INIT_AUTO_SPAWN 1\n' \
    >>"$SB/include/easynet_cli.h"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired auto-spawn ABI should exit 1 (got $rc)"

echo "test_check_ffi_abi_v4_header.sh: all cases passed"
