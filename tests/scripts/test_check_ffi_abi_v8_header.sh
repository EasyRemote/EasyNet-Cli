#!/usr/bin/env bash

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-ffi-abi-v8-header.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p \
        "$sandbox/include" \
        "$sandbox/src" \
        "$sandbox/docs/spec" \
        "$sandbox/sdk/conformance/fixtures" \
        "$sandbox/sdk/python/easynet_sdk/providers/runtime" \
        "$sandbox/sdk/python/tests" \
        "$sandbox/sdk/go"
    cp "$REPO_ROOT/include/easynet_cli.h" "$sandbox/include/easynet_cli.h"
    cp "$REPO_ROOT/include/easynet_cli.exports.v7" "$sandbox/include/easynet_cli.exports.v7"
    cp "$REPO_ROOT/include/easynet_cli.exports.v8" "$sandbox/include/easynet_cli.exports.v8"
    cp -R "$REPO_ROOT/src/ffi" "$sandbox/src/ffi"
    cp "$REPO_ROOT/docs/spec/ffi-abi-v8.md" "$sandbox/docs/spec/ffi-abi-v8.md"
    cp "$REPO_ROOT/sdk/conformance/fixtures/feature-discovery.v7.json" \
        "$sandbox/sdk/conformance/fixtures/feature-discovery.v7.json"
    cp "$REPO_ROOT/sdk/python/easynet_sdk/_cabi.py" "$sandbox/sdk/python/easynet_sdk/_cabi.py"
    cp "$REPO_ROOT/sdk/python/easynet_sdk/stream.py" "$sandbox/sdk/python/easynet_sdk/stream.py"
    cp "$REPO_ROOT/sdk/python/easynet_sdk/providers/runtime/direct.py" \
        "$sandbox/sdk/python/easynet_sdk/providers/runtime/direct.py"
    cp "$REPO_ROOT/sdk/python/tests/test_stream.py" "$sandbox/sdk/python/tests/test_stream.py"
    cp "$REPO_ROOT/sdk/python/tests/test_cabi.py" "$sandbox/sdk/python/tests/test_cabi.py"
    cp "$REPO_ROOT/sdk/go/cabi_runtime.go" "$sandbox/sdk/go/cabi_runtime.go"
    cp "$REPO_ROOT/sdk/go/cabi_callbacks.go" "$sandbox/sdk/go/cabi_callbacks.go"
    cp "$REPO_ROOT/sdk/go/cabi_runtime_test.go" "$sandbox/sdk/go/cabi_runtime_test.go"
    cp "$REPO_ROOT/sdk/go/stream.go" "$sandbox/sdk/go/stream.go"
    echo "$sandbox"
}

run_check() {
    local sandbox="$1"
    (
        cd "$sandbox"
        CHECK_FFI_ABI_V8_HEADER_ROOT="$sandbox" \
        EASYNET_FFI_DYLIB="${EASYNET_FFI_DYLIB:-$sandbox/not-built}" \
        bash "$SCRIPT"
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
run_check "$SB" >/dev/null || fail "clean v8 contract should pass"
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/#define RUNTIME_ABI_V8_EXTENSION_VERSION 8u/#define RUNTIME_ABI_V8_EXTENSION_VERSION 9u/' \
    "$SB/include/easynet_cli.h"
expect_failure "v8 extension version drift" "$SB"
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/runtime_invocation_stream_open_v8/runtime_invocation_stream_open_raw/' \
    "$SB/include/easynet_cli.h"
expect_failure "v8 header symbol drift" "$SB"
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/JSON pointers, v8 frame pointers/JSON pointers/' \
    "$SB/include/easynet_cli.h"
expect_failure "v8 borrowed payload ownership removed" "$SB"
rm -rf "$SB"

SB="$(make_sandbox)"
sed -i.bak '/runtime_invocation_stream_open_v8/d' "$SB/include/easynet_cli.exports.v8"
rm -f "$SB/include/easynet_cli.exports.v8.bak"
expect_failure "v8 allowlist missing raw symbol" "$SB"
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/"stream_binary_frame": true/"stream_binary_frame": false/' \
    "$SB/sdk/conformance/fixtures/feature-discovery.v7.json"
expect_failure "v8 feature discovery disabled" "$SB"
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/v8\.get\("symbol"\) == "runtime_invocation_stream_open_v8"/True/' \
    "$SB/sdk/python/easynet_sdk/_cabi.py"
expect_failure "Python ignores advertised v8 symbol" "$SB"
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/return observed\n        sequence = self\._next_sequence/sequence = self._next_sequence\n        self._next_sequence += 1\n        return sequence\n        sequence = self._next_sequence/' \
    "$SB/sdk/python/easynet_sdk/_cabi.py"
expect_failure "Python rewrites observed Runtime sequence" "$SB"
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/test_stream_rejects_duplicate_runtime_sequence/test_stream_accepts_duplicate_runtime_sequence/' \
    "$SB/sdk/python/tests/test_stream.py"
expect_failure "Python StreamHandle duplicate-sequence proof removed" "$SB"
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/isinstance\(raw, RawStreamPacket\)/isinstance(raw, bytes)/' \
    "$SB/sdk/python/easynet_sdk/_cabi.py"
expect_failure "Python routes binary v8 frames through legacy JSON repair" "$SB"
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/test_v8_callback_rejects_incompatible_frame_layout/test_v8_callback_accepts_incompatible_frame_layout/' \
    "$SB/sdk/python/tests/test_cabi.py"
expect_failure "Python exact v8 EOF proof removed" "$SB"
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/test_v8_callback_queue_overflow_is_carrier_error_not_runtime_frame/test_v8_callback_queue_overflow_invents_runtime_frame/' \
    "$SB/sdk/python/tests/test_cabi.py"
expect_failure "Python v8 overflow carrier-error proof removed" "$SB"
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/test_raw_stream_packet_rejects_noncanonical_state_and_object_fields/test_raw_stream_packet_accepts_noncanonical_state_and_object_fields/' \
    "$SB/sdk/python/tests/test_stream.py"
expect_failure "Python canonical v8 metadata type proof removed" "$SB"
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/v8\["symbol"\] == "runtime_invocation_stream_open_v8"/true/' \
    "$SB/sdk/go/cabi_runtime.go"
expect_failure "Go ignores advertised v8 symbol" "$SB"
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/if !packet\.hasBinary/if false/' "$SB/sdk/go/cabi_runtime.go"
expect_failure "Go accepts non-binary v8 callback packet" "$SB"
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/TestCABIV8TransportPreservesMalformedHeaderForFailClosedValidation/TestCABIV8TransportRepairsMalformedHeader/' \
    "$SB/sdk/go/cabi_runtime_test.go"
expect_failure "Go malformed v8 metadata proof removed" "$SB"
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/TestCABIV8CallbackRejectsIncompatibleFrameLayout/TestCABIV8CallbackAcceptsIncompatibleFrameLayout/' \
    "$SB/sdk/go/cabi_runtime_test.go"
expect_failure "Go exact v8 EOF proof removed" "$SB"
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/RemoteApp WebRTC/RemoteApp transport/' "$SB/docs/spec/ffi-abi-v8.md"
expect_failure "v8 product transport boundary removed" "$SB"
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/stream_v8_header_uses_canonical_wire_types/stream_v8_header_accepts_ambiguous_wire_types/' \
    "$SB/src/ffi/invocation/mod.rs"
expect_failure "v8 canonical metadata wire-type proof removed" "$SB"
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/stream_v8_header_rejects_noncanonical_state_name/stream_v8_header_accepts_noncanonical_state_name/' \
    "$SB/src/ffi/invocation/mod.rs"
expect_failure "v8 noncanonical state rejection proof removed" "$SB"
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
        done <"$SB/include/easynet_cli.exports.v8"
    } >"$C_SOURCE"
    if [[ "$(uname -s)" == "Darwin" ]]; then
        cc -dynamiclib -o "$LIB" "$C_SOURCE"
    else
        cc -shared -fPIC -o "$LIB" "$C_SOURCE"
    fi
    EASYNET_FFI_DYLIB="$LIB" run_check "$SB" >/dev/null \
        || fail "exact v8 dynamic-library exports should pass"

    sed -i.bak '/runtime_invocation_stream_open_v8/d' "$C_SOURCE"
    rm -f "$C_SOURCE.bak"
    if [[ "$(uname -s)" == "Darwin" ]]; then
        cc -dynamiclib -o "$LIB" "$C_SOURCE"
    else
        cc -shared -fPIC -o "$LIB" "$C_SOURCE"
    fi
    rc=0
    EASYNET_FFI_DYLIB="$LIB" run_check "$SB" >/dev/null 2>&1 || rc=$?
    [[ "$rc" == "1" ]] || fail "missing v8 dynamic-library export should exit 1"
    rm -rf "$SB"
fi

echo "test_check_ffi_abi_v8_header.sh: all cases passed"
