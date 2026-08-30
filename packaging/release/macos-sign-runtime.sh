#!/usr/bin/env bash
# Sign and verify the complete macOS EasyNet Runtime process set.
#
# Screen Recording is owned by the RemoteApp media host and Accessibility
# input is owned by the daemon. Both TCC decisions require a stable,
# certificate-backed designated requirement across product updates; Rust's
# linker-generated ad-hoc cdhash identity is not a release identity.

set -euo pipefail

usage() {
    cat <<'USAGE'
Usage:
  macos-sign-runtime.sh --stage-dir DIR [--verify-only]

Environment:
  EASYNET_MACOS_CODESIGN_IDENTITY  Developer ID Application identity. Required
                                   unless --verify-only is selected.
  EASYNET_MACOS_TEAM_ID            Exact 10-character Apple Team ID. Required.
USAGE
}

stage_dir=""
verify_only=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --stage-dir) stage_dir="${2:?missing value for --stage-dir}"; shift 2 ;;
        --verify-only) verify_only=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "macos-sign-runtime.sh: unknown argument: $1" >&2; usage >&2; exit 64 ;;
    esac
done

[[ "$(uname -s)" == "Darwin" ]] || {
    echo "macos-sign-runtime.sh: macOS signing requires Darwin" >&2
    exit 1
}
command -v codesign >/dev/null 2>&1 || {
    echo "macos-sign-runtime.sh: codesign is required" >&2
    exit 1
}
[[ -n "$stage_dir" && -d "$stage_dir" && "$stage_dir" != "/" ]] || {
    echo "macos-sign-runtime.sh: --stage-dir must name an existing bounded directory" >&2
    exit 1
}

team_id="${EASYNET_MACOS_TEAM_ID:-}"
identity="${EASYNET_MACOS_CODESIGN_IDENTITY:-}"
[[ "$team_id" =~ ^[A-Z0-9]{10}$ ]] || {
    echo "macos-sign-runtime.sh: EASYNET_MACOS_TEAM_ID must be an exact 10-character Apple Team ID" >&2
    exit 1
}
if [[ "$verify_only" -eq 0 && -z "$identity" ]]; then
    echo "macos-sign-runtime.sh: EASYNET_MACOS_CODESIGN_IDENTITY is required for release signing" >&2
    exit 1
fi

artifacts=(
    "libaxon_dendrite_bridge.dylib"
    "libeasynet_cli.dylib"
    "easynet-keyring"
    "easynet-remoteapp-native-host"
    "easynet-remoteapp-media-host.app"
    "easynet-daemon"
    "easynet"
)
identifiers=(
    "run.easynet.dendrite-bridge"
    "run.easynet.runtime-c-abi"
    "run.easynet.keyring"
    "run.easynet.remoteapp.native-host"
    "run.easynet.remoteapp.media-host"
    "run.easynet.daemon"
    "run.easynet.cli"
)

verify_artifact() {
    local path="$1"
    local identifier="$2"
    local details requirement
    codesign --verify --strict --verbose=2 "$path"
    details="$(codesign -dv --verbose=4 "$path" 2>&1)"
    grep -Fqx "Identifier=$identifier" <<<"$details" || {
        echo "macos-sign-runtime.sh: $path has the wrong identifier; expected $identifier" >&2
        exit 1
    }
    grep -Fqx "TeamIdentifier=$team_id" <<<"$details" || {
        echo "macos-sign-runtime.sh: $path is not signed by expected Team ID $team_id" >&2
        exit 1
    }
    grep -Eq '^CodeDirectory .*flags=.*\(.*runtime.*\)' <<<"$details" || {
        echo "macos-sign-runtime.sh: $path is missing hardened-runtime signing" >&2
        exit 1
    }
    requirement="$(codesign -d -r- "$path" 2>&1)"
    if grep -Fq '# designated => cdhash ' <<<"$requirement"; then
        echo "macos-sign-runtime.sh: $path retains an unstable ad-hoc cdhash identity" >&2
        exit 1
    fi
}

for index in "${!artifacts[@]}"; do
    path="$stage_dir/${artifacts[$index]}"
    identifier="${identifiers[$index]}"
    [[ -e "$path" ]] || {
        echo "macos-sign-runtime.sh: required staged artifact missing: $path" >&2
        exit 1
    }
    if [[ "$verify_only" -eq 0 ]]; then
        codesign --force --options runtime --timestamp \
            --identifier "$identifier" --sign "$identity" "$path"
    fi
    verify_artifact "$path" "$identifier"
done

echo "macos-sign-runtime.sh: ok (stable Team ID $team_id)"
