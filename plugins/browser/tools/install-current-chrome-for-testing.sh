#!/usr/bin/env bash
set -euo pipefail

METADATA_URL="https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json"

fail() {
  printf 'Chrome for Testing install failed: %s\n' "$1" >&2
  exit 1
}

for command_name in curl python3 unzip; do
  command -v "$command_name" >/dev/null 2>&1 \
    || fail "required command is unavailable: $command_name"
done

case "$(uname -s):$(uname -m)" in
  Darwin:arm64) platform="mac-arm64" ;;
  Darwin:x86_64) platform="mac-x64" ;;
  Linux:x86_64) platform="linux64" ;;
  *) fail "unsupported platform: $(uname -s) $(uname -m)" ;;
esac

chrome_root="${EASYNET_BROWSER_CHROME_ROOT:-}"
if [[ -z "$chrome_root" ]]; then
  chrome_root="$(python3 -c 'from pathlib import Path; print(Path.home() / ".easynet" / "browser" / "chrome")')"
fi
[[ "$chrome_root" == /* && "$chrome_root" != "/" ]] \
  || fail "EASYNET_BROWSER_CHROME_ROOT must be a non-root absolute path"
mkdir -p "$chrome_root"

work_dir="$(mktemp -d "$chrome_root/.install.XXXXXX")"
cleanup() {
  if [[ -n "${work_dir:-}" && -d "$work_dir" ]]; then
    rm -rf -- "$work_dir"
  fi
}
trap cleanup EXIT

metadata_path="$work_dir/versions.json"
curl --fail --location --silent --show-error \
  --retry 3 --connect-timeout 15 --max-time 120 \
  "$METADATA_URL" -o "$metadata_path"

selection="$(python3 - "$metadata_path" "$platform" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    metadata = json.load(handle)
stable = metadata["channels"]["Stable"]
for download in stable["downloads"]["chrome"]:
    if download["platform"] == sys.argv[2]:
        print(stable["version"])
        print(download["url"])
        break
else:
    raise SystemExit(f"no Stable Chrome download for {sys.argv[2]}")
PY
)"
version="${selection%%$'\n'*}"
download_url="${selection#*$'\n'}"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]] \
  || fail "official metadata returned an invalid version: $version"
[[ "$download_url" == https://storage.googleapis.com/chrome-for-testing-public/* ]] \
  || fail "official metadata returned an unexpected download origin"

target_dir="$chrome_root/$version"
if [[ -d "$target_dir" ]]; then
  printf 'Chrome for Testing %s is already installed at %s\n' "$version" "$target_dir"
  exit 0
fi
[[ ! -e "$target_dir" ]] || fail "install target already exists and is not a directory: $target_dir"

archive_path="$work_dir/chrome.zip"
curl --fail --location --silent --show-error \
  --retry 3 --connect-timeout 15 --max-time 600 \
  "$download_url" -o "$archive_path"
unzip -q "$archive_path" -d "$work_dir/unpacked"

case "$platform" in
  mac-*)
    unpacked_dir="$work_dir/unpacked/chrome-$platform"
    browser_binary="$unpacked_dir/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing"
    ;;
  linux64)
    unpacked_dir="$work_dir/unpacked/chrome-linux64"
    browser_binary="$unpacked_dir/chrome"
    ;;
esac
[[ -x "$browser_binary" ]] || fail "downloaded archive omitted the Chrome executable"
actual_version="$("$browser_binary" --version)"
[[ "$actual_version" == *"$version"* ]] \
  || fail "downloaded executable version mismatch: $actual_version"

mv "$unpacked_dir" "$target_dir"
printf 'Installed Chrome for Testing %s at %s\n' "$version" "$target_dir"
