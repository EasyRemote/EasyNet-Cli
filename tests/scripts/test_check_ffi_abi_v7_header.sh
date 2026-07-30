#!/usr/bin/env bash

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-ffi-abi-v6-header.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p "$sandbox/include" "$sandbox/src" "$sandbox/docs/spec"
    cp "$REPO_ROOT/include/easynet_cli.h" "$sandbox/include/easynet_cli.h"
    cp "$REPO_ROOT/include/easynet_cli.exports.v6" "$sandbox/include/easynet_cli.exports.v6"
    cp -R "$REPO_ROOT/src/ffi" "$sandbox/src/ffi"
    cp "$REPO_ROOT/docs/spec/ffi-abi-v6.md" "$sandbox/docs/spec/ffi-abi-v6.md"
    echo "$sandbox"
}

run_check() {
    local sandbox="$1"
    shift
    (
        cd "$sandbox"
        CHECK_FFI_ABI_V6_HEADER_ROOT="$sandbox" \
        EASYNET_FFI_DYLIB="${EASYNET_FFI_DYLIB:-$sandbox/not-built}" \
        bash "$SCRIPT" "$@"
    )
}

expect_failure() {
    local label="$1"
    local sandbox="$2"
    local rc=0
    run_check "$sandbox" >/dev/null 2>&1 || rc=$?
    [[ "$rc" == "1" ]] || fail "$label should exit 1 (got $rc)"
}

SB="$(make_sandbox)"
run_check "$SB" >/dev/null || fail "clean v6 contract should pass"
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/#define RUNTIME_ABI_VERSION 6u/#define RUNTIME_ABI_VERSION 4u/' \
    "$SB/include/easynet_cli.h"
expect_failure "ABI version drift" "$SB"
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/int32_t runtime_health/int32_t runtime_health_missing/' \
    "$SB/include/easynet_cli.h"
expect_failure "missing header declaration" "$SB"
rm -rf "$SB"

SB="$(make_sandbox)"
printf '\nint32_t easynet_identity_project_ura(void);\n' >>"$SB/include/easynet_cli.h"
expect_failure "unexpected product header declaration" "$SB"
rm -rf "$SB"

SB="$(make_sandbox)"
cat >>"$SB/src/ffi/mod.rs" <<'RS'
#[no_mangle]
pub extern "C" fn easynet_unreviewed_extra() -> i32 { 0 }
RS
expect_failure "unexpected Rust export" "$SB"
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/fn runtime_invocation_bidi_open/fn runtime_invocation_bidi_start/' \
    "$SB/src/ffi/invocation/mod.rs"
expect_failure "renamed Rust export" "$SB"
rm -rf "$SB"

SB="$(make_sandbox)"
printf 'runtime_string_free\n' >>"$SB/include/easynet_cli.exports.v6"
expect_failure "duplicate allowlist entry" "$SB"
rm -rf "$SB"

if command -v cc >/dev/null 2>&1 && command -v nm >/dev/null 2>&1; then
    SB="$(make_sandbox)"
    C_SOURCE="$SB/exports.c"
    LIB="$SB/libeasynet_cli.so"
    if [[ "$(uname -s)" == "Darwin" ]]; then
        LIB="$SB/libeasynet_cli.dylib"
    fi
    {
        while IFS= read -r symbol; do
            printf 'void %s(void) {}\n' "$symbol"
        done <"$SB/include/easynet_cli.exports.v6"
    } >"$C_SOURCE"
    if [[ "$(uname -s)" == "Darwin" ]]; then
        cc -dynamiclib -o "$LIB" "$C_SOURCE"
    else
        cc -shared -fPIC -o "$LIB" "$C_SOURCE"
    fi
    EASYNET_FFI_DYLIB="$LIB" run_check "$SB" >/dev/null \
        || fail "exact dynamic-library exports should pass"

    printf '\nvoid easynet_identity_project_ura(void) {}\n' >>"$C_SOURCE"
    if [[ "$(uname -s)" == "Darwin" ]]; then
        cc -dynamiclib -o "$LIB" "$C_SOURCE"
    else
        cc -shared -fPIC -o "$LIB" "$C_SOURCE"
    fi
    rc=0
    EASYNET_FFI_DYLIB="$LIB" run_check "$SB" >/dev/null 2>&1 || rc=$?
    [[ "$rc" == "1" ]] || fail "unexpected dynamic-library export should exit 1"
    rm -rf "$SB"
fi

echo "test_check_ffi_abi_v6_header.sh: all cases passed"
