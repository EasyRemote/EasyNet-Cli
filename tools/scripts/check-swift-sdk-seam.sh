#!/usr/bin/env bash
set -euo pipefail

ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
BUILD_DIR="$ROOT/target/swift-sdk-seam"

rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR"

sources_file="$BUILD_DIR/sources.txt"
find "$ROOT/sdk/swift/Sources" "$ROOT/sdk/swift/Tests" -name '*.swift' | sort >"$sources_file"
if [[ ! -s "$sources_file" ]]; then
  echo "check-swift-sdk-seam: missing Swift sources" >&2
  exit 1
fi

swiftc -warnings-as-errors @"$sources_file" -o "$BUILD_DIR/runtime-core-seam-test"
"$BUILD_DIR/runtime-core-seam-test"

address_terms='U''RI|U''ri|u''ri'
if grep -R -nE "\\b($address_terms)\\b|axon\\.v1|protobuf|easynet\\.run/axon" "$ROOT/sdk/swift" >/tmp/easynet-swift-sdk-seam-grep.$$ 2>/dev/null; then
  cat /tmp/easynet-swift-sdk-seam-grep.$$ >&2
  rm -f /tmp/easynet-swift-sdk-seam-grep.$$
  echo "check-swift-sdk-seam: Swift seam leaked forbidden naming or Axon/proto symbols" >&2
  exit 1
fi
rm -f /tmp/easynet-swift-sdk-seam-grep.$$

echo "check-swift-sdk-seam ok"
