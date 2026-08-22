#!/usr/bin/env bash
# Additive contract gate for the libeasynet_cli C ABI v8 raw stream extension.

set -euo pipefail

ROOT="${CHECK_FFI_ABI_V8_HEADER_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

HEADER="include/easynet_cli.h"
V7_ALLOWLIST="include/easynet_cli.exports.v7"
V8_ALLOWLIST="include/easynet_cli.exports.v8"
FEATURE_FIXTURE="sdk/conformance/fixtures/feature-discovery.v7.json"
EXPECTED_V7_COUNT=56
EXPECTED_V8_COUNT=57
V8_SYMBOL="runtime_invocation_stream_open_v8"
violations=0
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

record_violation() {
    echo "ERROR: $1" >&2
    echo "$2" >&2
    violations=$((violations + 1))
}

require_file() {
    if [[ ! -f "$1" ]]; then
        record_violation "required file missing" "$1"
        return 1
    fi
}

require_literal() {
    if ! grep -Fq "$2" "$1"; then
        record_violation "required literal missing from $1" "$2"
    fi
}

extract_header_symbols() {
    python3 - "$1" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8")
text = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
text = re.sub(r"//[^\n]*", "", text)
for symbol in re.findall(r"\b(runtime_[A-Za-z0-9_]+)\s*\(", text):
    print(symbol)
PY
}

extract_source_symbols() {
    python3 - "src/ffi" <<'PY'
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
text = "\n".join(path.read_text(encoding="utf-8") for path in sorted(root.rglob("*.rs")))
explicit = re.findall(
    r'#\[no_mangle\]\s*pub\s+(?:unsafe\s+)?extern\s+"C"\s+fn\s+(runtime_[A-Za-z0-9_]+)',
    text,
)
generated = re.findall(r'builder_string_setter!\s*\(\s*(runtime_[A-Za-z0-9_]+)', text)
for symbol in explicit + generated:
    print(symbol)
PY
}

if require_file "$V7_ALLOWLIST"; then
    if ! LC_ALL=C sort -c "$V7_ALLOWLIST" 2>/dev/null; then
        record_violation "v7 allowlist must be sorted" "$V7_ALLOWLIST"
    fi
    v7_count="$(wc -l <"$V7_ALLOWLIST" | tr -d ' ')"
    v7_unique="$(sort -u "$V7_ALLOWLIST" | wc -l | tr -d ' ')"
    if [[ "$v7_count" != "$EXPECTED_V7_COUNT" || "$v7_unique" != "$EXPECTED_V7_COUNT" ]]; then
        record_violation "v7 allowlist count changed" "lines=$v7_count unique=$v7_unique"
    fi
fi

if require_file "$V8_ALLOWLIST"; then
    if ! LC_ALL=C sort -c "$V8_ALLOWLIST" 2>/dev/null; then
        record_violation "v8 allowlist must be sorted" "$V8_ALLOWLIST"
    fi
    v8_count="$(wc -l <"$V8_ALLOWLIST" | tr -d ' ')"
    v8_unique="$(sort -u "$V8_ALLOWLIST" | wc -l | tr -d ' ')"
    if [[ "$v8_count" != "$EXPECTED_V8_COUNT" || "$v8_unique" != "$EXPECTED_V8_COUNT" ]]; then
        record_violation "v8 allowlist must contain exactly $EXPECTED_V8_COUNT unique symbols" \
            "lines=$v8_count unique=$v8_unique"
    fi
    comm -23 "$V7_ALLOWLIST" "$V8_ALLOWLIST" >"$tmp/v8-missing-v7" || true
    comm -13 "$V7_ALLOWLIST" "$V8_ALLOWLIST" >"$tmp/v8-additions" || true
    if [[ -s "$tmp/v8-missing-v7" ]]; then
        record_violation "v8 allowlist must include every v7 symbol" "$(cat "$tmp/v8-missing-v7")"
    fi
    if [[ "$(cat "$tmp/v8-additions")" != "$V8_SYMBOL" ]]; then
        record_violation "v8 allowlist must add only the raw stream symbol" "$(cat "$tmp/v8-additions")"
    fi
fi

if require_file "$HEADER"; then
    require_literal "$HEADER" "#define RUNTIME_ABI_VERSION 7u"
    require_literal "$HEADER" "#define RUNTIME_ABI_V8_EXTENSION_VERSION 8u"
    require_literal "$HEADER" "typedef void (*RuntimeInvocationStreamV8Callback)"
    require_literal "$HEADER" "$V8_SYMBOL"
    extract_header_symbols "$HEADER" | LC_ALL=C sort >"$tmp/header.symbols"
    if ! diff -u "$V8_ALLOWLIST" "$tmp/header.symbols" >"$tmp/header.diff"; then
        record_violation "header declarations must match v8 allowlist" "$(cat "$tmp/header.diff")"
    fi
fi

extract_source_symbols | LC_ALL=C sort >"$tmp/source.symbols"
if ! diff -u "$V8_ALLOWLIST" "$tmp/source.symbols" >"$tmp/source.diff"; then
    record_violation "Rust exported source symbols must match v8 allowlist" "$(cat "$tmp/source.diff")"
fi

if require_file "$FEATURE_FIXTURE"; then
    require_literal "$FEATURE_FIXTURE" '"abi_version": 7'
    require_literal "$FEATURE_FIXTURE" '"abi_extensions"'
    require_literal "$FEATURE_FIXTURE" '"stream_raw_payload": true'
    require_literal "$FEATURE_FIXTURE" '"symbol": "runtime_invocation_stream_open_v8"'
    require_literal "$FEATURE_FIXTURE" '"stream_raw_payload_v8": true'
fi

if ! rg -q "InvocationStreamV8Callback" src/ffi/invocation/mod.rs; then
    record_violation "Rust FFI must define v8 callback target" "src/ffi/invocation/mod.rs"
fi
if ! rg -q "StreamCallbackDelivery::V8" src/ffi/invocation/mod.rs; then
    record_violation "Rust stream delivery must have v8 raw payload variant" "src/ffi/invocation/mod.rs"
fi
if ! rg -q "RawStreamPacket" sdk/python/easynet_sdk/_cabi.py; then
    record_violation "Python C ABI provider must expose raw packet bridge" "sdk/python/easynet_sdk/_cabi.py"
fi
if ! rg -q "runtime_cabi_call_stream_open_v8" sdk/go/cabi_runtime.go; then
    record_violation "Go C ABI provider must bind v8 raw stream open" "sdk/go/cabi_runtime.go"
fi
if ! rg -q "easynetGoStreamV8Callback" sdk/go/cabi_callbacks.go; then
    record_violation "Go C ABI provider must expose v8 raw stream callback" "sdk/go/cabi_callbacks.go"
fi
if ! rg -q "func \\(e StreamEvent\\) PayloadBytes\\(\\) \\[\\]byte" sdk/go/stream.go; then
    record_violation "Go stream facade must expose raw payload bytes" "sdk/go/stream.go"
fi
if ! rg -q "TestCABIRuntimeProviderFallsBackToV7StreamOpen" sdk/go/cabi_runtime_test.go; then
    record_violation "Go C ABI provider must test v7 fallback" "sdk/go/cabi_runtime_test.go"
fi
if rg -q "RawStreamPacket|_stream_chunk_packet" sdk/python/easynet_sdk/providers/runtime/direct.py; then
    record_violation "direct runtime provider must remain canonical JSON/base64" \
        "sdk/python/easynet_sdk/providers/runtime/direct.py"
fi

if [[ "$violations" -ne 0 ]]; then
    echo "FAILED: $violations v8 ABI extension violation(s)." >&2
    exit 1
fi

echo "ok (generic C ABI v8 extension: raw stream transport symbol is additive)"
