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
        "required_command in cargo cmake cc" \
        "cargo build --locked" \
        "--lib" \
        "--bin easynet-keyring" \
        "include/easynet_cli.h" \
        "include/easynet_cli.exports.v7" \
        "include/easynet_cli.exports.v8" \
        "include/easynet_cli.exports.v9" \
        "docs/spec/ffi-abi-v7.md" \
        "docs/spec/ffi-abi-v8.md" \
        "docs/spec/ffi-abi-v9.md" \
        "easynet-keyring" \
        "easynet-remoteapp-native-host" \
        "easynet-remoteapp-media-host" \
        "libeasynet_cli" \
        "libaxon_dendrite_bridge"
    do
        require_literal "packaging/release/build-release-tarball.sh" "$literal"
    done
    require_literal "packaging/release/build-release-tarball.sh" 'bash "$script_dir/macos-sign-runtime.sh" --stage-dir "$stage_dir"'
    require_literal "packaging/release/build-release-tarball.sh" 'bash "$script_dir/constrain-c-abi-exports.sh"'
    require_literal "packaging/release/build-release-tarball.sh" 'EASYNET_FFI_DYLIB="$stage_dir/libeasynet_cli.${lib_ext}"'
fi

if require_file "packaging/release/constrain-c-abi-exports.sh"; then
    require_literal "packaging/release/constrain-c-abi-exports.sh" 'nmedit -s'
    require_literal "packaging/release/constrain-c-abi-exports.sh" 'objcopy --localize-symbols='
    require_literal "packaging/release/constrain-c-abi-exports.sh" 'diff -u "$allowlist" "$tmp/actual"'
fi

if require_file "packaging/release/macos-sign-runtime.sh"; then
    for literal in \
        "EASYNET_MACOS_CODESIGN_IDENTITY" \
        "EASYNET_MACOS_TEAM_ID" \
        "run.easynet.daemon" \
        "run.easynet.remoteapp.media-host" \
        "run.easynet.runtime-c-abi" \
        "--options runtime" \
        "--timestamp" \
        "# designated => cdhash"
    do
        require_literal "packaging/release/macos-sign-runtime.sh" "$literal"
    done
fi

if require_file "packaging/release/install.sh"; then
    for literal in \
        "INCLUDE_DIR=\"/usr/local/include/easynet\"" \
        "LIB_DIR=\"/usr/local/lib\"" \
        "DOC_DIR=\"/usr/local/share/doc/easynet\"" \
        "easynet-keyring" \
        "easynet-remoteapp-native-host" \
        "easynet-remoteapp-media-host" \
        "libeasynet_cli" \
        "include/easynet_cli.h" \
        "easynet_cli.exports.v7" \
        "easynet_cli.exports.v8" \
        "easynet_cli.exports.v9" \
        "ffi-abi-v7.md" \
        "ffi-abi-v8.md" \
        "ffi-abi-v9.md"
    do
        require_literal "packaging/release/install.sh" "$literal"
    done
fi

if require_file "packaging/release/dev-install-local.sh"; then
    for literal in \
        "--bin easynet-keyring" \
        "easynet-keyring" \
        "easynet-remoteapp-native-host" \
        "easynet-remoteapp-media-host" \
        '$keyring_bin' \
        '"$install_dir/easynet-keyring"'
    do
        require_literal "packaging/release/dev-install-local.sh" "$literal"
    done
fi

if require_file "packaging/release/build-windows-cli.ps1"; then
    require_literal "packaging/release/build-windows-cli.ps1" 'Require-Command "cmake"'
    require_literal "packaging/release/build-windows-cli.ps1" '"--locked"'
    require_literal "packaging/release/build-windows-cli.ps1" '"--lib"'
    require_literal "packaging/release/build-windows-cli.ps1" 'easynet_cli.dll'
    require_literal "packaging/release/build-windows-cli.ps1" 'Required C ABI contract file missing'
    require_literal "packaging/release/build-windows-cli.ps1" "docs\spec\ffi-abi-v8.md"
    require_literal "packaging/release/build-windows-cli.ps1" "ffi-abi-v8.md"
    require_literal "packaging/release/build-windows-cli.ps1" "docs\spec\ffi-abi-v9.md"
    require_literal "packaging/release/build-windows-cli.ps1" "ffi-abi-v9.md"
    require_literal "packaging/release/build-windows-cli.ps1" "easynet-remoteapp-native-host.exe"
    require_literal "packaging/release/build-windows-cli.ps1" "easynet-remoteapp-media-host.exe"
fi

if require_file "packaging/release/install.ps1"; then
    require_literal "packaging/release/install.ps1" 'easynet_cli.dll'
    require_literal "packaging/release/install.ps1" 'easynet_cli.exports.v9'
    require_literal "packaging/release/install.ps1" 'ffi-abi-v9.md'
fi

if require_file ".github/workflows/release-runtime.yml"; then
    for literal in \
        "build-windows:" \
        "x86_64-pc-windows-gnu" \
        "actions-setup-cmake@v2" \
        "setup-mingw@v2" \
        "Verify bundled Opus has no runtime DLL dependency"
    do
        require_literal ".github/workflows/release-runtime.yml" "$literal"
    done
    require_literal ".github/workflows/release-runtime.yml" "Verify Windows v9 C ABI exports"
    for literal in \
        "Import macOS Developer ID identity" \
        "EASYNET_MACOS_CERTIFICATE_P12_BASE64" \
        "EASYNET_MACOS_CODESIGN_IDENTITY" \
        "EASYNET_MACOS_TEAM_ID" \
        "security set-key-partition-list"
    do
        require_literal ".github/workflows/release-runtime.yml" "$literal"
    done
fi

if require_file "packaging/release/e2e-release-install.sh"; then
    for literal in \
        "easynet-keyring" \
        "easynet-remoteapp-native-host" \
        "easynet-remoteapp-media-host" \
        "libeasynet_cli" \
        "include/easynet_cli.h" \
        "include/easynet_cli.exports.v7" \
        "include/easynet_cli.exports.v8" \
        "include/easynet_cli.exports.v9" \
        "docs/spec/ffi-abi-v7.md" \
        "docs/spec/ffi-abi-v8.md" \
        "docs/spec/ffi-abi-v9.md" \
        "#define RUNTIME_ABI_VERSION 7u" \
        "EASYNET_FFI_DYLIB" \
        "c abi:"
    do
        require_literal "packaging/release/e2e-release-install.sh" "$literal"
    done
fi

if require_file "include/easynet_cli.exports.v9"; then
    require_literal "include/easynet_cli.exports.v9" "runtime_abi_version"
    require_literal "include/easynet_cli.exports.v9" "runtime_invocation_stream_open_v9"
    require_literal "include/easynet_cli.exports.v9" "runtime_buffer_lease_retain_v9"
    require_literal "include/easynet_cli.exports.v9" "runtime_buffer_lease_release_v9"
    if [[ "$(wc -l < include/easynet_cli.exports.v9 | tr -d ' ')" != "60" ]]; then
        record_violation "v9 export allowlist must contain exactly 60 symbols" "include/easynet_cli.exports.v9"
    fi
    comm -23 include/easynet_cli.exports.v8 include/easynet_cli.exports.v9 > "$tmp/v9-missing-v8" || true
    if [[ -s "$tmp/v9-missing-v8" ]]; then
        record_violation "v9 export allowlist must include every v8 symbol" "$(cat "$tmp/v9-missing-v8")"
    fi
    comm -13 include/easynet_cli.exports.v8 include/easynet_cli.exports.v9 > "$tmp/v9-added" || true
    if [[ "$(cat "$tmp/v9-added")" != "runtime_buffer_lease_release_v9
runtime_buffer_lease_retain_v9
runtime_invocation_stream_open_v9" ]]; then
        record_violation "v9 export allowlist has an invalid additive set" "$(cat "$tmp/v9-added")"
    fi
fi

require_file "plugins/remote-desktop/native-host/Cargo.toml" || true
require_file "plugins/remote-desktop/native-host/src/main.rs" || true
require_file "plugins/remote-desktop/media-host/Cargo.toml" || true
require_file "plugins/remote-desktop/media-host/src/main.rs" || true

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
    require_literal "docs/spec/ffi-abi-v7.md" "include/easynet_cli.exports.v9"
fi

if require_file "docs/spec/ffi-abi-v9.md"; then
    require_literal "docs/spec/ffi-abi-v9.md" "runtime_invocation_stream_open_v9"
    require_literal "docs/spec/ffi-abi-v9.md" "runtime_buffer_lease_release_v9"
    require_literal "docs/spec/ffi-abi-v9.md" "runtime_feature_discovery"
    require_literal "docs/spec/ffi-abi-v9.md" "RemoteApp WebRTC"
fi

if require_file "docs/spec/ffi-abi-v8.md"; then
    require_literal "docs/spec/ffi-abi-v8.md" "runtime_invocation_stream_open_v8"
    require_literal "docs/spec/ffi-abi-v8.md" "runtime_feature_discovery"
    require_literal "docs/spec/ffi-abi-v8.md" "RemoteApp WebRTC"
fi

if [[ "$violations" -eq 0 ]]; then
    echo "ok (release package contract is clean)"
    exit 0
fi

echo "FAILED: $violations violation(s)."
exit 1
