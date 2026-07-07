#!/usr/bin/env bash
set -euo pipefail

ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"

if [[ ! -f "$ROOT/sdk/swift/Package.swift" ]]; then
  echo "check-swift-sdk-seam: missing Swift Package manifest" >&2
  exit 1
fi
if [[ ! -d "$ROOT/sdk/swift/Sources/EasyNetDaemonSDK" || ! -d "$ROOT/sdk/swift/Tests/EasyNetDaemonSDKTests" ]]; then
  echo "check-swift-sdk-seam: missing Swift sources" >&2
  exit 1
fi

swift test --package-path "$ROOT/sdk/swift" -Xswiftc -warnings-as-errors

address_terms='U''RI|U''ri|u''ri'
if grep -R -nE "\\b($address_terms)\\b|axon\\.v1|protobuf|easynet\\.run/axon" \
  "$ROOT/sdk/swift/Package.swift" \
  "$ROOT/sdk/swift/README.md" \
  "$ROOT/sdk/swift/Sources" \
  "$ROOT/sdk/swift/Tests" >/tmp/easynet-swift-sdk-seam-grep.$$ 2>/dev/null; then
  cat /tmp/easynet-swift-sdk-seam-grep.$$ >&2
  rm -f /tmp/easynet-swift-sdk-seam-grep.$$
  echo "check-swift-sdk-seam: Swift seam leaked forbidden naming or Axon/proto symbols" >&2
  exit 1
fi
rm -f /tmp/easynet-swift-sdk-seam-grep.$$

echo "check-swift-sdk-seam ok"
