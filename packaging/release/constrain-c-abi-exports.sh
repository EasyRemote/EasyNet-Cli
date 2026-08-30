#!/usr/bin/env bash
# Reduce the staged libeasynet_cli dynamic surface to the exact v9 allowlist.

set -euo pipefail

usage() {
    echo "Usage: constrain-c-abi-exports.sh --library PATH --allowlist PATH" >&2
}

library=""
allowlist=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --library) library="${2:?missing library path}"; shift 2 ;;
        --allowlist) allowlist="${2:?missing allowlist path}"; shift 2 ;;
        *) usage; exit 64 ;;
    esac
done

[[ -f "$library" && -f "$allowlist" ]] || {
    usage
    exit 1
}
LC_ALL=C sort -c "$allowlist"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

case "$(uname -s)" in
    Darwin)
        command -v nmedit >/dev/null 2>&1 || {
            echo "constrain-c-abi-exports.sh: nmedit is required on macOS" >&2
            exit 1
        }
        sed 's/^/_/' "$allowlist" >"$tmp/platform-allowlist"
        nmedit -s "$tmp/platform-allowlist" "$library"
        nm -gU "$library" | awk '{print $NF}' | sed 's/^_//' | LC_ALL=C sort -u >"$tmp/actual"
        ;;
    Linux)
        command -v objcopy >/dev/null 2>&1 || {
            echo "constrain-c-abi-exports.sh: objcopy is required on Linux" >&2
            exit 1
        }
        nm -D --defined-only "$library" | awk '{print $NF}' | sed 's/@.*//' | LC_ALL=C sort -u >"$tmp/before"
        comm -13 "$allowlist" "$tmp/before" >"$tmp/localize"
        if [[ -s "$tmp/localize" ]]; then
            objcopy --localize-symbols="$tmp/localize" "$library"
        fi
        nm -D --defined-only "$library" | awk '{print $NF}' | sed 's/@.*//' | LC_ALL=C sort -u >"$tmp/actual"
        ;;
    *)
        echo "constrain-c-abi-exports.sh: unsupported host" >&2
        exit 1
        ;;
esac

diff -u "$allowlist" "$tmp/actual"
echo "constrain-c-abi-exports.sh: ok ($(wc -l <"$tmp/actual" | tr -d ' ') symbols)"
