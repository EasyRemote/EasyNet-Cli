#!/usr/bin/env bash
# check-release-package-contract.sh
# =================================
#
# Static CI gate for the EasyNet-Cli release package shape.
#
# packaging/release/build-release-tarball.sh, packaging/release/install.sh,
# and packaging/release/e2e-release-install.sh must agree on every required
# artefact. This catches drift before a release tarball reaches a real
# installer or a language binding misses part of the generic ABI v6 contract.

set -euo pipefail

ROOT="${CHECK_RELEASE_PACKAGE_CONTRACT_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

echo "== check-release-package-contract.sh =="

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
        "include/easynet_cli.exports.v6" \
        "docs/spec/ffi-abi-v6.md" \
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
        "easynet_cli.exports.v6" \
        "ffi-abi-v6.md"
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
        "include/easynet_cli.exports.v6" \
        "docs/spec/ffi-abi-v6.md" \
        "#define RUNTIME_ABI_VERSION 6u" \
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
    require_literal "include/easynet_cli.h" "#define RUNTIME_ABI_VERSION 6u"
fi

if require_file "include/easynet_cli.exports.v6"; then
    require_literal "include/easynet_cli.exports.v6" "runtime_abi_version"
    if [[ "$(wc -l < include/easynet_cli.exports.v6 | tr -d ' ')" != "55" ]]; then
        record_violation "v6 export allowlist must contain exactly 55 symbols" "include/easynet_cli.exports.v6"
    fi
fi

if require_file "docs/spec/ffi-abi-v6.md"; then
    require_literal "docs/spec/ffi-abi-v6.md" "include/easynet_cli.h"
    require_literal "docs/spec/ffi-abi-v6.md" "include/easynet_cli.exports.v6"
fi

if [[ "$violations" -eq 0 ]]; then
    echo "ok (release package contract is clean)"
    exit 0
fi

echo "FAILED: $violations violation(s)."
exit 1
