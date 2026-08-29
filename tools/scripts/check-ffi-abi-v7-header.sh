#!/usr/bin/env bash
# Exact contract gate for the generic libeasynet_cli C ABI v7 surface.

set -euo pipefail

ROOT="${CHECK_FFI_ABI_V7_HEADER_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

HEADER="include/easynet_cli.h"
ALLOWLIST="include/easynet_cli.exports.v7"
ALLOWLIST_V8="include/easynet_cli.exports.v8"
ALLOWLIST_LATEST="include/easynet_cli.exports.v9"
SPEC="docs/spec/ffi-abi-v7.md"
RELEASE_TARBALL_SCRIPT="packaging/release/build-release-tarball.sh"
RELEASE_INSTALL_E2E_SCRIPT="packaging/release/e2e-release-install.sh"
RELEASE_INSTALL_SCRIPT="packaging/release/install.sh"
WINDOWS_RELEASE_SCRIPT="packaging/release/build-windows-cli.ps1"
EXPECTED_COUNT=56
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

extract_product_prefixed_header_symbols() {
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
    r'#\[no_mangle\]\s*pub\s+(?:unsafe\s+)?extern\s+"C"\s+fn\s+(runtime_[A-Za-z0-9_]+)',
    text,
)
generated = re.findall(
    r'builder_string_setter!\s*\(\s*(runtime_[A-Za-z0-9_]+)',
    text,
)
for symbol in explicit + generated:
    print(symbol)
PY
}

extract_product_prefixed_source_symbols() {
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
        record_violation "$label differs from exact v7 allowlist" "$(cat "$tmp/$label.diff")"
    fi
}

compare_v7_surface_with_extensions() {
    local label="$1"
    local actual="$2"
    local sorted_latest="$tmp/latest.symbols"
    if [[ -f "$ALLOWLIST_LATEST" ]]; then
        LC_ALL=C sort "$ALLOWLIST_LATEST" >"$sorted_latest"
    elif [[ -f "$ALLOWLIST_V8" ]]; then
        LC_ALL=C sort "$ALLOWLIST_V8" >"$sorted_latest"
    else
        cp "$ALLOWLIST" "$sorted_latest"
    fi
    comm -23 "$ALLOWLIST" "$actual" >"$tmp/$label.missing-v7" || true
    comm -23 "$actual" "$sorted_latest" >"$tmp/$label.unexpected" || true
    if [[ -s "$tmp/$label.missing-v7" || -s "$tmp/$label.unexpected" ]]; then
        record_violation "$label is not v7 plus declared ABI extensions" \
            "missing_v7:\n$(cat "$tmp/$label.missing-v7")\nunexpected:\n$(cat "$tmp/$label.unexpected")"
    fi
}

exported_symbols() {
    local lib="$1"
    case "$(uname -s)" in
        Darwin)
            nm -gU "$lib" 2>/dev/null | awk '{print $NF}' | sed 's/^_//'
            ;;
        Linux)
            nm -D --defined-only "$lib" 2>/dev/null | awk '{print $NF}' | sed 's/@.*//'
            ;;
        *)
            return 1
            ;;
    esac
}

product_prefixed_exported_symbols() {
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
        record_violation "v7 allowlist must be sorted" "$ALLOWLIST"
    fi
    allowlist_count="$(wc -l <"$ALLOWLIST" | tr -d ' ')"
    unique_count="$(sort -u "$ALLOWLIST" | wc -l | tr -d ' ')"
    if [[ "$allowlist_count" != "$EXPECTED_COUNT" || "$unique_count" != "$EXPECTED_COUNT" ]]; then
        record_violation "v7 allowlist must contain exactly $EXPECTED_COUNT unique symbols" \
            "lines=$allowlist_count unique=$unique_count"
    fi
    if grep -Ev '^runtime_[a-z0-9_]+$' "$ALLOWLIST" >"$tmp/invalid-allowlist"; then
        record_violation "v7 allowlist contains invalid symbol names" "$(cat "$tmp/invalid-allowlist")"
    fi
    if grep -E '^easynet_' "$ALLOWLIST" >"$tmp/product-allowlist"; then
        record_violation "v7 allowlist must not contain product-prefixed symbols" "$(cat "$tmp/product-allowlist")"
    fi
fi
if require_file "$ALLOWLIST_V8"; then
    if ! LC_ALL=C sort -c "$ALLOWLIST_V8" 2>/dev/null; then
        record_violation "v8 allowlist must be sorted" "$ALLOWLIST_V8"
    fi
    if ! comm -23 "$ALLOWLIST" "$ALLOWLIST_V8" | sed '/^$/d' >"$tmp/v8-missing-v7"; then
        true
    fi
    if [[ -s "$tmp/v8-missing-v7" ]]; then
        record_violation "v8 allowlist must include every v7 baseline symbol" "$(cat "$tmp/v8-missing-v7")"
    fi
    require_literal "$ALLOWLIST_V8" "runtime_invocation_stream_open_v8"
fi
if require_file "$ALLOWLIST_LATEST"; then
    if ! LC_ALL=C sort -c "$ALLOWLIST_LATEST" 2>/dev/null; then
        record_violation "latest extension allowlist must be sorted" "$ALLOWLIST_LATEST"
    fi
    comm -23 "$ALLOWLIST_V8" "$ALLOWLIST_LATEST" >"$tmp/v9-missing-v8" || true
    if [[ -s "$tmp/v9-missing-v8" ]]; then
        record_violation "v9 allowlist must include every v8 symbol" "$(cat "$tmp/v9-missing-v8")"
    fi
    require_literal "$ALLOWLIST_LATEST" "runtime_invocation_stream_open_v9"
    require_literal "$ALLOWLIST_LATEST" "runtime_buffer_lease_retain_v9"
    require_literal "$ALLOWLIST_LATEST" "runtime_buffer_lease_release_v9"
fi

if require_file "$HEADER"; then
    require_literal "$HEADER" "#define RUNTIME_ABI_VERSION 7u"
    require_literal "$HEADER" "#define RUNTIME_ABI_V8_EXTENSION_VERSION 8u"
    require_literal "$HEADER" "#define RUNTIME_ABI_V9_EXTENSION_VERSION 9u"
    if command -v cc >/dev/null 2>&1; then
        if ! printf '#include "include/easynet_cli.h"\n' | cc -I. -x c -fsyntax-only - >/dev/null 2>&1; then
            record_violation "C compiler rejects v7 header" "$HEADER"
        fi
    else
        record_violation "C compiler unavailable" "cc is required"
    fi
    extract_header_symbols "$HEADER" | LC_ALL=C sort >"$tmp/header.symbols"
    compare_v7_surface_with_extensions "header declarations" "$tmp/header.symbols"
    extract_product_prefixed_header_symbols "$HEADER" | LC_ALL=C sort >"$tmp/header.product-symbols"
    if [[ -s "$tmp/header.product-symbols" ]]; then
        record_violation "v7 header must not declare product-prefixed C ABI symbols" "$(cat "$tmp/header.product-symbols")"
    fi
fi

if require_file "src/ffi/mod.rs"; then
    require_literal "src/ffi/mod.rs" "pub const RUNTIME_ABI_VERSION: u32 = 7;"
    extract_source_symbols | LC_ALL=C sort >"$tmp/source.symbols"
    compare_v7_surface_with_extensions "Rust exported source symbols" "$tmp/source.symbols"
    extract_product_prefixed_source_symbols | LC_ALL=C sort >"$tmp/source.product-symbols"
    if [[ -s "$tmp/source.product-symbols" ]]; then
        record_violation "v7 Rust exports must not include product-prefixed C ABI symbols" "$(cat "$tmp/source.product-symbols")"
    fi
fi

for error_pair in \
    "RUNTIME_OK 0" "ERR_GENERIC 1" "ERR_NULL_POINTER 2" \
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
    require_literal "$SPEC" "include/easynet_cli.exports.v7"
    require_literal "$SPEC" "include/easynet_cli.exports.v8"
    require_literal "$SPEC" "include/easynet_cli.exports.v9"
    require_literal "$SPEC" 'exactly `56`'
    require_literal "$SPEC" 'exactly `57`'
    require_literal "$SPEC" 'exactly `60`'
    require_literal "$SPEC" "runtime_invocation_stream_open_v8"
    require_literal "$SPEC" 'ABI version: `7`'
fi

for release_script in \
    "$RELEASE_TARBALL_SCRIPT" \
    "$RELEASE_INSTALL_E2E_SCRIPT" \
    "$RELEASE_INSTALL_SCRIPT" \
    "$WINDOWS_RELEASE_SCRIPT"
do
    if require_file "$release_script"; then
        require_literal "$release_script" "easynet_cli.exports.v8"
        require_literal "$release_script" "easynet_cli.exports.v9"
    fi
done

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
        compare_v7_surface_with_extensions "dynamic-library exports" "$tmp/dylib.symbols"
        product_prefixed_exported_symbols "$lib" | LC_ALL=C sort >"$tmp/dylib.product-symbols"
        if [[ -s "$tmp/dylib.product-symbols" ]]; then
            record_violation "dynamic library must not export product-prefixed C ABI symbols" "$(cat "$tmp/dylib.product-symbols")"
        fi
    fi
elif [[ "${EASYNET_FFI_REQUIRE_DYLIB:-0}" == "1" ]]; then
    record_violation "dynamic library required but missing" "${lib:-unsupported platform}"
fi

if [[ "$violations" -ne 0 ]]; then
    echo "FAILED: $violations v7 ABI contract violation(s)." >&2
    exit 1
fi

echo "ok (generic C ABI v7: exactly $EXPECTED_COUNT symbols)"
