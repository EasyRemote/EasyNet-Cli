#!/usr/bin/env bash
# Stage the macOS RemoteApp media host as the application identity that owns
# ScreenCaptureKit. System Settings does not admit a flat Unix executable into
# Screen & System Audio Recording. The daemon launches this bundle through
# LaunchServices and transfers its bounded private lanes over SCM_RIGHTS.

set -euo pipefail

usage() {
    echo "Usage: macos-stage-remoteapp-media-host.sh --binary PATH --output-dir DIR" >&2
}

binary=""
output_dir=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --binary) binary="${2:?missing value for --binary}"; shift 2 ;;
        --output-dir) output_dir="${2:?missing value for --output-dir}"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "macos-stage-remoteapp-media-host.sh: unknown argument: $1" >&2; usage; exit 64 ;;
    esac
done

[[ -f "$binary" && -x "$binary" ]] || {
    echo "macos-stage-remoteapp-media-host.sh: executable media host missing: $binary" >&2
    exit 1
}
[[ -n "$output_dir" && "$output_dir" != "/" ]] || {
    echo "macos-stage-remoteapp-media-host.sh: --output-dir must be a bounded directory" >&2
    exit 1
}

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cli_root="$(cd "$script_dir/../.." && pwd)"
plist="$cli_root/plugins/remote-desktop/media-host/Info.plist"
[[ -f "$plist" ]] || {
    echo "macos-stage-remoteapp-media-host.sh: canonical Info.plist missing: $plist" >&2
    exit 1
}

app="$output_dir/easynet-remoteapp-media-host.app"
executable="$app/Contents/MacOS/easynet-remoteapp-media-host"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
install -m 755 "$binary" "$executable"
install -m 644 "$plist" "$app/Contents/Info.plist"

if command -v plutil >/dev/null 2>&1; then
    plutil -lint "$app/Contents/Info.plist" >/dev/null
fi

echo "$app"
