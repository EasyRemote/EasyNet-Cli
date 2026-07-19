#!/usr/bin/env bash
# Exact contract gate for the generic libeasynet_cli C ABI v5 surface.

set -euo pipefail

ROOT="${CHECK_FFI_ABI_V5_HEADER_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

HEADER="include/easynet_cli.h"
ALLOWLIST="include/easynet_cli.exports.v5"
SPEC="docs/spec/ffi-abi-v5.md"
EXPECTED_COUNT=55
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
for symbol in re.findall(r"\b(easynet_[A-Za-z0-9_]+)\s*\(", text):
    print(symbol)
PY
}

extract_source_symbols() {
    python3 - "src/ffi" <<'PY'
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
text = "\n".join(
    path.read_text(encoding="utf-8") for path in sorted(root.rglob("*.rs"))
)
explicit = re.findall(
    r'#\[no_mangle\]\s*pub\s+(?:unsafe\s+)?extern\s+"C"\s+fn\s+(easynet_[A-Za-z0-9_]+)',
    text,
)
generated = re.findall(
    r'builder_string_setter!\s*\(\s*(easynet_[A-Za-z0-9_]+)',
    text,
)
for symbol in explicit + generated:
    print(symbol)
PY
}

compare_exact() {
    local label="$1"
    local actual="$2"
    if ! diff -u "$ALLOWLIST" "$actual" >"$tmp/$label.diff"; then
        record_violation "$label differs from exact v5 allowlist" "$(cat "$tmp/$label.diff")"
    fi
}

exported_symbols() {
    local lib="$1"
    case "$(uname -s)" in
        Darwin)
            nm -gU "$lib" 2>/dev/null | awk '{print $NF}' | sed 's/^_//' | grep '^easynet_' || true
            ;;
        Linux)
            nm -D --defined-only "$lib" 2>/dev/null | awk '{print $NF}' | sed 's/^_//' | grep '^easynet_' || true
            ;;
        *)
            return 1
            ;;
    esac
}

if require_file "$ALLOWLIST"; then
    if ! LC_ALL=C sort -c "$ALLOWLIST" 2>/dev/null; then
        record_violation "v5 allowlist must be sorted" "$ALLOWLIST"
    fi
    allowlist_count="$(wc -l <"$ALLOWLIST" | tr -d ' ')"
    unique_count="$(sort -u "$ALLOWLIST" | wc -l | tr -d ' ')"
    if [[ "$allowlist_count" != "$EXPECTED_COUNT" || "$unique_count" != "$EXPECTED_COUNT" ]]; then
        record_violation "v5 allowlist must contain exactly $EXPECTED_COUNT unique symbols" \
            "lines=$allowlist_count unique=$unique_count"
    fi
    if grep -Ev '^easynet_[a-z0-9_]+$' "$ALLOWLIST" >"$tmp/invalid-allowlist"; then
        record_violation "v5 allowlist contains invalid symbol names" "$(cat "$tmp/invalid-allowlist")"
    fi
fi

if require_file "$HEADER"; then
    require_literal "$HEADER" "#define EASYNET_ABI_VERSION 5u"
    if command -v cc >/dev/null 2>&1; then
        if ! printf '#include "include/easynet_cli.h"\n' | cc -I. -x c -fsyntax-only - >/dev/null 2>&1; then
            record_violation "C compiler rejects v5 header" "$HEADER"
        fi
    else
        record_violation "C compiler unavailable" "cc is required"
    fi
    extract_header_symbols "$HEADER" | LC_ALL=C sort >"$tmp/header.symbols"
    compare_exact "header declarations" "$tmp/header.symbols"
fi

if require_file "src/ffi/mod.rs"; then
    require_literal "src/ffi/mod.rs" "pub const EASYNET_ABI_VERSION: u32 = 5;"
    extract_source_symbols | LC_ALL=C sort >"$tmp/source.symbols"
    compare_exact "Rust exported source symbols" "$tmp/source.symbols"
fi

for error_pair in \
    "EASYNET_OK 0" "ERR_GENERIC 1" "ERR_NULL_POINTER 2" \
    "ERR_INVALID_UTF8 3" "ERR_INVALID_HANDLE 4" "ERR_NOT_INITIALIZED 5" \
    "ERR_ALREADY_INIT 6" "ERR_DAEMON_DOWN 7" "ERR_VERSION_INCOMPATIBLE 8" \
    "ERR_ABILITY_FAILED 9" "ERR_NOT_IMPLEMENTED 10" "ERR_INVALID_ARG 11" \
    "ERR_PERMISSION_DENIED 12" "ERR_NOT_FOUND 13" "ERR_CANCELLED 14" \
    "ERR_PROTOCOL 15" "ERR_TIMEOUT 16"
do
    name="${error_pair% *}"
    value="${error_pair##* }"
    require_literal "$HEADER" "#define $name $value"
    require_literal "src/ffi/errors/mod.rs" "pub const $name: i32 = $value;"
done

for retired_module in \
    admin_gateway authority compatibility directory events host_binding identity \
    mission profile_json publication receipt surface wrappers
do
    if [[ -e "src/ffi/$retired_module" ]]; then
        record_violation "domain FFI module still exists" "src/ffi/$retired_module"
    fi
done

if require_file "$SPEC"; then
    require_literal "$SPEC" "include/easynet_cli.h"
    require_literal "$SPEC" "include/easynet_cli.exports.v5"
    require_literal "$SPEC" 'exactly `55`'
    require_literal "$SPEC" 'ABI version: `5`'
fi

lib="${EASYNET_FFI_DYLIB:-}"
if [[ -z "$lib" ]]; then
    case "$(uname -s)" in
        Darwin) lib="target/debug/libeasynet_cli.dylib" ;;
        Linux) lib="target/debug/libeasynet_cli.so" ;;
        *) lib="" ;;
    esac
fi
if [[ -n "$lib" && -f "$lib" ]]; then
    if ! command -v nm >/dev/null 2>&1; then
        record_violation "nm unavailable for exact export check" "$lib"
    else
        exported_symbols "$lib" | LC_ALL=C sort >"$tmp/dylib.symbols"
        compare_exact "dynamic-library exports" "$tmp/dylib.symbols"
    fi
elif [[ "${EASYNET_FFI_REQUIRE_DYLIB:-0}" == "1" ]]; then
    record_violation "dynamic library required but missing" "${lib:-unsupported platform}"
fi

if [[ "$violations" -ne 0 ]]; then
    echo "FAILED: $violations v5 ABI contract violation(s)." >&2
    exit 1
fi

echo "ok (generic C ABI v5: exactly $EXPECTED_COUNT symbols)"
