#!/usr/bin/env bash
# Additive contract gate for the libeasynet_cli C ABI v9 payload lease.

set -euo pipefail

ROOT="${CHECK_FFI_ABI_V9_HEADER_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

HEADER="include/easynet_cli.h"
V8="include/easynet_cli.exports.v8"
V9="include/easynet_cli.exports.v9"
SPEC="docs/spec/ffi-abi-v9.md"
FIXTURE="sdk/conformance/fixtures/feature-discovery.v7.json"
violations=0
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

fail() {
    echo "ERROR: $1" >&2
    echo "$2" >&2
    violations=$((violations + 1))
}

require_file() {
    [[ -f "$1" ]] || { fail "required file missing" "$1"; return 1; }
}

require_literal() {
    grep -Fq "$2" "$1" || fail "required literal missing from $1" "$2"
}

extract_header_symbols() {
    python3 - "$1" <<'PY'
import re, sys
from pathlib import Path
text = Path(sys.argv[1]).read_text(encoding="utf-8")
text = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
text = re.sub(r"//[^\n]*", "", text)
print("\n".join(sorted(set(re.findall(r"\b(runtime_[A-Za-z0-9_]+)\s*\(", text)))))
PY
}

extract_source_symbols() {
    python3 - "src/ffi" <<'PY'
import re, sys
from pathlib import Path
text = "\n".join(p.read_text(encoding="utf-8") for p in sorted(Path(sys.argv[1]).rglob("*.rs")))
symbols = re.findall(r'#\[no_mangle\]\s*pub\s+(?:unsafe\s+)?extern\s+"C"\s+fn\s+(runtime_[A-Za-z0-9_]+)', text)
symbols += re.findall(r'builder_string_setter!\s*\(\s*(runtime_[A-Za-z0-9_]+)', text)
print("\n".join(sorted(set(symbols))))
PY
}

if require_file "$V8" && require_file "$V9"; then
    LC_ALL=C sort -c "$V8" 2>/dev/null || fail "v8 allowlist is not sorted" "$V8"
    LC_ALL=C sort -c "$V9" 2>/dev/null || fail "v9 allowlist is not sorted" "$V9"
    [[ "$(wc -l < "$V8" | tr -d ' ')" == 57 ]] || fail "v8 symbol count changed" "$V8"
    [[ "$(wc -l < "$V9" | tr -d ' ')" == 60 ]] || fail "v9 must contain exactly 60 symbols" "$V9"
    comm -23 "$V8" "$V9" >"$tmp/missing-v8" || true
    [[ ! -s "$tmp/missing-v8" ]] || fail "v9 does not contain every v8 symbol" "$(cat "$tmp/missing-v8")"
    comm -13 "$V8" "$V9" >"$tmp/additions" || true
    expected=$'runtime_buffer_lease_release_v9\nruntime_buffer_lease_retain_v9\nruntime_invocation_stream_open_v9'
    [[ "$(cat "$tmp/additions")" == "$expected" ]] || fail "v9 additive set is invalid" "$(cat "$tmp/additions")"
fi

if require_file "$HEADER"; then
    for literal in \
        "#define RUNTIME_ABI_V9_EXTENSION_VERSION 9u" \
        "#define RUNTIME_STREAM_FRAME_V9_ABI_VERSION 9u" \
        "typedef struct RuntimeBufferLeaseV9" \
        "typedef struct RuntimeInvocationStreamFrameV9" \
        "typedef void (*RuntimeInvocationStreamV9Callback)" \
        "runtime_invocation_stream_open_v9" \
        "runtime_buffer_lease_retain_v9" \
        "runtime_buffer_lease_release_v9"
    do
        require_literal "$HEADER" "$literal"
    done
    printf '#include "include/easynet_cli.h"\n' | cc -I. -x c -fsyntax-only - >/dev/null 2>&1 || fail "C compiler rejects the ABI header" "$HEADER"
    extract_header_symbols "$HEADER" >"$tmp/header"
    diff -u "$V9" "$tmp/header" >"$tmp/header.diff" || fail "header does not match v9 allowlist" "$(cat "$tmp/header.diff")"
fi

extract_source_symbols >"$tmp/source"
diff -u "$V9" "$tmp/source" >"$tmp/source.diff" || fail "Rust exports do not match v9 allowlist" "$(cat "$tmp/source.diff")"

if require_file "$FIXTURE"; then
    for literal in \
        '"stream_buffer_lease": true' \
        '"open_symbol": "runtime_invocation_stream_open_v9"' \
        '"retain_symbol": "runtime_buffer_lease_retain_v9"' \
        '"release_symbol": "runtime_buffer_lease_release_v9"' \
        '"stream_buffer_lease_v9": true'
    do
        require_literal "$FIXTURE" "$literal"
    done
fi

if require_file "$SPEC"; then
    for literal in \
        "include/easynet_cli.exports.v9" \
        "runtime_feature_discovery" \
        "runtime_buffer_lease_retain_v9" \
        "runtime_buffer_lease_release_v9" \
        "64 outstanding" \
        "256 MiB" \
        "RuntimeHandle shutdown" \
        "RemoteApp WebRTC"
    do
        require_literal "$SPEC" "$literal"
    done
fi

for literal in \
    "struct BufferLeaseRegistry" \
    "STREAM_V9_MAX_OUTSTANDING_LEASES" \
    "STREAM_V9_MAX_OUTSTANDING_BYTES"
do
    require_literal "src/ffi/invocation/buffer_lease.rs" "$literal"
done

for literal in \
    "purge_buffer_leases_for_binding(owner)" \
    "v9_payload_moves_from_vec_and_remains_valid_until_final_release" \
    "v9_oversized_payload_projects_an_explicit_error_before_eof" \
    "v9_lease_bound_backpressures_and_stream_close_wakes_waiters" \
    "stream_close_waits_for_inflight_callback_and_suppresses_late_eof" \
    "v9_queue_budget_remains_held_by_delivered_lease" \
    "v9_empty_payload_rejects_closed_stream_before_canonicalizing_empty_lease" \
    "stream_registry_enforces_per_handle_limit" \
    "MAX_ACTIVE_STREAMS_PER_OWNER" \
    "MAX_ACTIVE_STREAMS_GLOBAL"
do
    require_literal "src/ffi/invocation/mod.rs" "$literal"
done

lib="${EASYNET_FFI_DYLIB:-}"
if [[ -n "$lib" && -f "$lib" ]]; then
    case "$(uname -s)" in
        Darwin) nm -gU "$lib" | awk '{print $NF}' | sed 's/^_//' | LC_ALL=C sort >"$tmp/dylib" ;;
        Linux) nm -D --defined-only "$lib" | awk '{print $NF}' | sed 's/@.*//' | LC_ALL=C sort -u >"$tmp/dylib" ;;
    esac
    diff -u "$V9" "$tmp/dylib" >"$tmp/dylib.diff" || fail "dynamic exports do not match v9" "$(cat "$tmp/dylib.diff")"
fi

if [[ "$violations" -ne 0 ]]; then
    echo "FAILED: $violations v9 ABI contract violation(s)." >&2
    exit 1
fi

echo "ok (generic C ABI v9: bounded payload lease is additive)"
