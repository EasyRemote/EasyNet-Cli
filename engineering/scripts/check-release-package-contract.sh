#!/usr/bin/env bash
# check-release-package-contract.sh
# =================================
#
# Static CI gate for the EasyNet-Cli release package shape.
#
# build-release-tarball.sh, install.sh, and e2e-release-install.sh
# must agree on every required artefact. This catches drift before a
# release tarball reaches a real installer or a language binding misses
# the ABI v3 header.

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

if require_file "scripts/build-release-tarball.sh"; then
    for literal in \
        "--bin easynet-keyring" \
        "include/easynet_cli.h" \
        "docs/spec/ffi-abi-v3.md" \
        "easynet-keyring" \
        "libaxon_dendrite_bridge"
    do
        require_literal "scripts/build-release-tarball.sh" "$literal"
    done
fi

if require_file "install.sh"; then
    for literal in \
        "INCLUDE_DIR=\"/usr/local/include/easynet\"" \
        "DOC_DIR=\"/usr/local/share/doc/easynet\"" \
        "easynet-keyring" \
        "include/easynet_cli.h" \
        "ffi-abi-v3.md"
    do
        require_literal "install.sh" "$literal"
    done
fi

if require_file "scripts/e2e-release-install.sh"; then
    for literal in \
        "easynet-keyring" \
        "include/easynet_cli.h" \
        "docs/spec/ffi-abi-v3.md" \
        "#define EASYNET_ABI_VERSION 3u" \
        "c abi:"
    do
        require_literal "scripts/e2e-release-install.sh" "$literal"
    done
fi

if require_file "include/easynet_cli.h"; then
    require_literal "include/easynet_cli.h" "#define EASYNET_ABI_VERSION 3u"
fi

if require_file "docs/spec/ffi-abi-v3.md"; then
    require_literal "docs/spec/ffi-abi-v3.md" "include/easynet_cli.h"
fi

if [[ "$violations" -eq 0 ]]; then
    echo "ok (release package contract is clean)"
    exit 0
fi

echo "FAILED: $violations violation(s)."
exit 1
