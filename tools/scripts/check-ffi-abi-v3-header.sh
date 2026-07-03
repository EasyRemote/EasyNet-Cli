#!/usr/bin/env bash
# check-ffi-abi-v3-header.sh
# ===========================
#
# CI gate for the libeasynet_cli ABI v3 contract.
#
# The Rust FFI implementation owns behavior. include/easynet_cli.h is
# the language-binding contract. This script catches drift where Rust
# changes exported symbols, ABI version, or error codes but the
# binding-facing header/spec are left stale.

set -euo pipefail

ROOT="${CHECK_FFI_ABI_V3_HEADER_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

HEADER="include/easynet_cli.h"
SPEC="docs/spec/ffi-abi-v3.md"

echo "== check-ffi-abi-v3-header.sh =="

violations=0

record_violation() {
    local title="$1"
    local detail="$2"
    echo "ERROR: $title"
    echo "$detail"
    violations=$((violations + 1))
}

require_file() {
    local file="$1"
    if [[ ! -f "$file" ]]; then
        record_violation "required file missing" "$file"
        return 1
    fi
    return 0
}

require_absent_file() {
    local file="$1"
    if [[ -e "$file" ]]; then
        record_violation "retired file still present" "$file"
    fi
}

require_literal() {
    local file="$1"
    local literal="$2"
    if ! grep -Fq "$literal" "$file"; then
        record_violation "required literal missing from $file" "$literal"
    fi
}

require_absent_literal() {
    local file="$1"
    local literal="$2"
    if [[ -f "$file" ]] && grep -Fq "$literal" "$file"; then
        record_violation "retired literal present in $file" "$literal"
    fi
}

require_source_literal() {
    local literal="$1"
    if ! grep -R -Fq "$literal" src/ffi 2>/dev/null; then
        record_violation "required Rust FFI literal missing" "$literal"
    fi
}

expected_symbols=(
    easynet_abi_version
    easynet_last_error
    easynet_string_free
    easynet_init
    easynet_shutdown
    easynet_daemon_start
    easynet_daemon_stop
    easynet_daemon_status
    easynet_daemon_invocation_endpoint
    easynet_daemon_open_client
    easynet_invocation_invoke
    easynet_invocation_stream_open
    easynet_invocation_stream_cancel
    easynet_invocation_bidi_open
    easynet_invocation_bidi_send
    easynet_invocation_bidi_close
    easynet_invocation_bidi_cancel
)

retired_symbols=(
    easynet_ability_invoke
    easynet_ability_subscribe
    easynet_subscription_cancel
)

exported_symbols() {
    local lib="$1"
    case "$(uname -s)" in
        Darwin)
            nm -gU "$lib" 2>/dev/null | awk '{print $NF}' | sed 's/^_//'
            ;;
        Linux)
            nm -D --defined-only "$lib" 2>/dev/null | awk '{print $NF}' | sed 's/^_//'
            ;;
        *)
            return 1
            ;;
    esac
}

check_exported_symbols_if_built() {
    local lib=""
    case "$(uname -s)" in
        Darwin) lib="target/debug/libeasynet_cli.dylib" ;;
        Linux) lib="target/debug/libeasynet_cli.so" ;;
        *) return 0 ;;
    esac

    if [[ ! -f "$lib" ]]; then
        if [[ "${EASYNET_FFI_REQUIRE_DYLIB:-0}" == "1" ]]; then
            record_violation "cdylib artifact missing for exported-symbol check" "$lib"
        fi
        return 0
    fi
    if ! command -v nm >/dev/null 2>&1; then
        record_violation "nm unavailable for exported-symbol check" "nm is required when $lib exists"
        return 0
    fi

    local symbols
    symbols="$(exported_symbols "$lib" || true)"
    if [[ -z "$symbols" ]]; then
        record_violation "could not read exported symbols from cdylib" "$lib"
        return 0
    fi
    for symbol in "${expected_symbols[@]}"; do
        if ! grep -Fxq "$symbol" <<<"$symbols"; then
            record_violation "cdylib does not export required ABI symbol" "$symbol"
        fi
    done
    for symbol in "${retired_symbols[@]}"; do
        if grep -Fxq "$symbol" <<<"$symbols"; then
            record_violation "cdylib still exports retired ability+args ABI symbol" "$symbol"
        fi
    done
}

if require_file "$HEADER"; then
    if command -v cc >/dev/null 2>&1; then
        if ! printf '#include "include/easynet_cli.h"\n' \
            | cc -I. -x c -fsyntax-only - >/dev/null 2>&1
        then
            record_violation "C compiler rejects include/easynet_cli.h" \
                "cc -I. -x c -fsyntax-only failed"
        fi
    else
        record_violation "C compiler unavailable" "cc is required to validate include/easynet_cli.h"
    fi

    require_literal "$HEADER" "#define EASYNET_ABI_VERSION 3u"

    for error_pair in \
        "EASYNET_OK 0" \
        "ERR_GENERIC 1" \
        "ERR_NULL_POINTER 2" \
        "ERR_INVALID_UTF8 3" \
        "ERR_INVALID_HANDLE 4" \
        "ERR_NOT_INITIALIZED 5" \
        "ERR_ALREADY_INIT 6" \
        "ERR_DAEMON_DOWN 7" \
        "ERR_VERSION_INCOMPATIBLE 8" \
        "ERR_ABILITY_FAILED 9" \
        "ERR_NOT_IMPLEMENTED 10" \
        "ERR_INVALID_ARG 11" \
        "ERR_PERMISSION_DENIED 12" \
        "ERR_NOT_FOUND 13" \
        "ERR_CANCELLED 14" \
        "ERR_PROTOCOL 15" \
        "ERR_TIMEOUT 16"
    do
        name="${error_pair% *}"
        value="${error_pair##* }"
        require_literal "$HEADER" "#define $name $value"
        require_literal "src/ffi/errors/mod.rs" "pub const $name: i32 = $value;"
    done

    for typedef in \
        "typedef uint64_t EasynetHandle;" \
        "typedef uint64_t EasynetDaemonHandle;" \
        "typedef uint64_t EasynetInvocationStreamId;" \
        "typedef uint64_t EasynetInvocationBidiId;" \
        "typedef void (*EasynetInvocationStreamCallback)(" \
        "typedef void (*EasynetInvocationBidiCallback)("
    do
        require_literal "$HEADER" "$typedef"
    done

    for retired in \
        "typedef uint64_t EasynetSubscriptionId;" \
        "typedef void (*EasynetFrameCallback)(" \
        "easynet_ability_invoke" \
        "easynet_ability_subscribe" \
        "easynet_subscription_cancel"
    do
        require_absent_literal "$HEADER" "$retired"
    done

    for header_symbol in "${expected_symbols[@]}"; do
        symbol="fn $header_symbol"
        require_literal "$HEADER" "$header_symbol"
        require_source_literal "$symbol"
    done

    for retired in "${retired_symbols[@]}"; do
        if grep -R -Fq "fn $retired" src/ffi 2>/dev/null; then
            record_violation "Rust FFI still exports retired ability+args ABI symbol" "$retired"
        fi
    done

    check_exported_symbols_if_built

    retired_auto_spawn="$(
        grep -nE 'EasynetInitMode|EASYNET_INIT_AUTO_SPAWN|AUTO_SPAWN|auto_spawn' "$HEADER" || true
    )"
    if [[ -n "$retired_auto_spawn" ]]; then
        record_violation "header exposes retired auto-spawn initialization ABI" "$retired_auto_spawn"
    fi
fi

require_absent_file "src/ffi/ability.rs"

if require_file "src/ffi/mod.rs"; then
    require_literal "src/ffi/mod.rs" "pub const EASYNET_ABI_VERSION: u32 = 3;"
    require_absent_literal "src/ffi/mod.rs" "pub mod ability;"
fi

if require_file "$SPEC"; then
    require_literal "$SPEC" "include/easynet_cli.h"
    require_literal "$SPEC" "ERR_INVALID_ARG"
    require_literal "$SPEC" "easynet_invocation_bidi_open"
    require_literal "$SPEC" "ability+args symbols are not exported"
fi

for source in src/ffi/errors/mod.rs src/ffi/mod.rs; do
    [[ -f "$source" ]] || continue
    bad_legacy="$(
        grep -nE 'EasynetInitMode|EASYNET_INIT_AUTO_SPAWN|AUTO_SPAWN|auto_spawn' "$source" || true
    )"
    if [[ -n "$bad_legacy" ]]; then
        record_violation "$source reintroduces retired auto-spawn initialization ABI" "$bad_legacy"
    fi
done

if [[ "$violations" -eq 0 ]]; then
    echo "ok (FFI ABI v3 header contract is clean)"
    exit 0
fi

echo "FAILED: $violations violation(s)."
exit 1
