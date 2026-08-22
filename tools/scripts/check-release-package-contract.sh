#!/usr/bin/env bash
# check-release-package-contract.sh
# =================================
#
# Static CI gate for the EasyNet-Cli release package shape.
#
# packaging/release/build-release-tarball.sh, packaging/release/install.sh,
# and packaging/release/e2e-release-install.sh must agree on every required
# artefact. This catches drift before a release tarball reaches a real
# installer or a language binding misses part of the generic ABI v7 contract
# and its feature-detected v8 raw-stream extension allowlist.

set -euo pipefail

ROOT="${CHECK_RELEASE_PACKAGE_CONTRACT_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

echo "== check-release-package-contract.sh =="

violations=0
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

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

require_literal() {
    local file="$1"
    local literal="$2"
    if ! grep -Fq -- "$literal" "$file"; then
        record_violation "required literal missing from $file" "$literal"
    fi
}

forbid_literal() {
    local file="$1"
    local literal="$2"
    if grep -Fq -- "$literal" "$file"; then
        record_violation "forbidden literal present in $file" "$literal"
    fi
}

if require_file "packaging/release/build-release-tarball.sh"; then
    for literal in \
        "--bin easynet-keyring" \
        "include/easynet_cli.h" \
        "include/easynet_cli.exports.v7" \
        "include/easynet_cli.exports.v8" \
        "docs/spec/ffi-abi-v7.md" \
        "easynet-keyring" \
        "libaxon_dendrite_bridge"
    do
        require_literal "packaging/release/build-release-tarball.sh" "$literal"
    done
fi

if require_file "packaging/release/install.sh"; then
    for literal in \
        "INCLUDE_DIR=\"/usr/local/include/easynet\"" \
        "DOC_DIR=\"/usr/local/share/doc/easynet\"" \
        "easynet-keyring" \
        "include/easynet_cli.h" \
        "easynet_cli.exports.v7" \
        "easynet_cli.exports.v8" \
        "ffi-abi-v7.md"
    do
        require_literal "packaging/release/install.sh" "$literal"
    done
fi

if require_file "packaging/release/dev-install-local.sh"; then
    for literal in \
        "--bin easynet-keyring" \
        "easynet-keyring" \
        '$keyring_bin' \
        '"$install_dir/easynet-keyring"'
    do
        require_literal "packaging/release/dev-install-local.sh" "$literal"
    done
fi

if require_file "packaging/release/e2e-release-install.sh"; then
    for literal in \
        "easynet-keyring" \
        "include/easynet_cli.h" \
        "include/easynet_cli.exports.v7" \
        "include/easynet_cli.exports.v8" \
        "docs/spec/ffi-abi-v7.md" \
        "#define RUNTIME_ABI_VERSION 7u" \
        "c abi:"
    do
        require_literal "packaging/release/e2e-release-install.sh" "$literal"
    done
fi

if require_file "packaging/release/e2e-release-flow.sh"; then
    require_literal "packaging/release/e2e-release-flow.sh" 'bash "$script_dir/e2e-release-install.sh"'
    forbid_literal "packaging/release/e2e-release-flow.sh" "e2e-release-packaging/release/install.sh"
fi

if require_file "include/easynet_cli.h"; then
    require_literal "include/easynet_cli.h" "#define RUNTIME_ABI_VERSION 7u"
fi

if require_file "include/easynet_cli.exports.v7"; then
    require_literal "include/easynet_cli.exports.v7" "runtime_abi_version"
    if [[ "$(wc -l < include/easynet_cli.exports.v7 | tr -d ' ')" != "56" ]]; then
        record_violation "v7 export allowlist must contain exactly 56 symbols" "include/easynet_cli.exports.v7"
    fi
fi

if require_file "include/easynet_cli.exports.v8"; then
    require_literal "include/easynet_cli.exports.v8" "runtime_abi_version"
    require_literal "include/easynet_cli.exports.v8" "runtime_invocation_stream_open_v8"
    if [[ "$(wc -l < include/easynet_cli.exports.v8 | tr -d ' ')" != "57" ]]; then
        record_violation "v8 export allowlist must contain exactly 57 symbols" "include/easynet_cli.exports.v8"
    fi
    if ! comm -23 include/easynet_cli.exports.v7 include/easynet_cli.exports.v8 | sed '/^$/d' > "$tmp/v8-missing-v7"; then
        true
    fi
    if [[ -s "$tmp/v8-missing-v7" ]]; then
        record_violation "v8 export allowlist must include every v7 symbol" "$(cat "$tmp/v8-missing-v7")"
    fi
    if ! comm -13 include/easynet_cli.exports.v7 include/easynet_cli.exports.v8 > "$tmp/v8-added"; then
        true
    fi
    if [[ "$(cat "$tmp/v8-added")" != "runtime_invocation_stream_open_v8" ]]; then
        record_violation "v8 export allowlist must add only runtime_invocation_stream_open_v8" "$(cat "$tmp/v8-added")"
    fi
fi

if require_file "docs/spec/ffi-abi-v7.md"; then
    require_literal "docs/spec/ffi-abi-v7.md" "include/easynet_cli.h"
    require_literal "docs/spec/ffi-abi-v7.md" "include/easynet_cli.exports.v7"
    require_literal "docs/spec/ffi-abi-v7.md" "include/easynet_cli.exports.v8"
fi

if [[ "$violations" -eq 0 ]]; then
    echo "ok (release package contract is clean)"
    exit 0
fi

echo "FAILED: $violations violation(s)."
exit 1
