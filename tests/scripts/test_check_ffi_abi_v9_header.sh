#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CHECK="$ROOT/tools/scripts/check-ffi-abi-v9-header.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

make_sandbox() {
    local dir
    dir="$(mktemp -d)"
    mkdir -p "$dir/include" "$dir/docs/spec" "$dir/src" "$dir/sdk/conformance/fixtures"
    cp "$ROOT/include/easynet_cli.h" "$dir/include/"
    cp "$ROOT/include/easynet_cli.exports.v8" "$dir/include/"
    cp "$ROOT/include/easynet_cli.exports.v9" "$dir/include/"
    cp "$ROOT/docs/spec/ffi-abi-v9.md" "$dir/docs/spec/"
    cp -R "$ROOT/src/ffi" "$dir/src/ffi"
    cp "$ROOT/sdk/conformance/fixtures/feature-discovery.v7.json" "$dir/sdk/conformance/fixtures/"
    echo "$dir"
}

run_check() {
    CHECK_FFI_ABI_V9_HEADER_ROOT="$1" EASYNET_FFI_DYLIB="" bash "$CHECK"
}

expect_failure() {
    local label="$1" dir="$2" rc=0
    run_check "$dir" >/dev/null 2>&1 || rc=$?
    [[ "$rc" == 1 ]] || fail "$label should fail (got $rc)"
}

SB="$(make_sandbox)"
run_check "$SB" >/dev/null || fail "clean v9 contract should pass"
rm -rf "$SB"

SB="$(make_sandbox)"
sed -i.bak '/runtime_buffer_lease_release_v9/d' "$SB/include/easynet_cli.exports.v9"
rm -f "$SB/include/easynet_cli.exports.v9.bak"
expect_failure "missing release symbol" "$SB"
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/#define RUNTIME_STREAM_FRAME_V9_ABI_VERSION 9u/#define RUNTIME_STREAM_FRAME_V9_ABI_VERSION 8u/' "$SB/include/easynet_cli.h"
expect_failure "v9 layout version drift" "$SB"
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/STREAM_V9_MAX_OUTSTANDING_LEASES/STREAM_V9_UNBOUNDED_LEASES/g' "$SB/src/ffi/invocation/buffer_lease.rs"
expect_failure "lease-count bound removed" "$SB"
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/MAX_ACTIVE_STREAMS_PER_OWNER/UNBOUNDED_STREAMS_PER_OWNER/g' "$SB/src/ffi/invocation/mod.rs"
expect_failure "per-handle stream bound removed" "$SB"
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/stream_close_waits_for_inflight_callback_and_suppresses_late_eof/stream_close_returns_before_callback/g' "$SB/src/ffi/invocation/mod.rs"
expect_failure "callback quiescence test removed" "$SB"
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/"stream_buffer_lease": true/"stream_buffer_lease": false/' "$SB/sdk/conformance/fixtures/feature-discovery.v7.json"
expect_failure "v9 feature disabled" "$SB"
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/RemoteApp WebRTC/RemoteApp generic stream/' "$SB/docs/spec/ffi-abi-v9.md"
expect_failure "RemoteApp boundary removed" "$SB"
rm -rf "$SB"

echo "test_check_ffi_abi_v9_header.sh: all cases passed"
