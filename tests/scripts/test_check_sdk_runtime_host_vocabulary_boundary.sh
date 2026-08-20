#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CHECK="$ROOT/tools/scripts/check-sdk-runtime-host-vocabulary-boundary.sh"

fail() {
  echo "test_check_sdk_runtime_host_vocabulary_boundary: $*" >&2
  exit 1
}

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p \
  "$SB/sdk/go" \
  "$SB/sdk/python/easynet_sdk"

for source in \
  sdk/go/addressing.go \
  sdk/go/authorized_runtime_session.go \
  sdk/go/authority.go \
  sdk/go/bidi.go \
  sdk/go/client.go \
  sdk/go/direct_runtime.go \
  sdk/go/directory.go \
  sdk/go/native_runtime_cabi.go \
  sdk/go/runtime.go \
  sdk/python/easynet_sdk/client.py \
  sdk/python/easynet_sdk/transport.py
do
  mkdir -p "$SB/$(dirname "$source")"
  cp "$ROOT/$source" "$SB/$source"
done

SDK_RUNTIME_HOST_VOCABULARY_ROOT="$SB" bash "$CHECK" >/dev/null

echo '// daemon lifecycle leak' >> "$SB/sdk/go/runtime.go"
if SDK_RUNTIME_HOST_VOCABULARY_ROOT="$SB" bash "$CHECK" >/tmp/sdk-runtime-host-vocab.out 2>&1; then
  fail "gate accepted daemon vocabulary in generic Go runtime source"
fi
grep -Fq "generic SDK runtime-host sources preserve daemon vocabulary" /tmp/sdk-runtime-host-vocab.out \
  || fail "gate failure did not explain generic SDK daemon vocabulary"

echo "test_check_sdk_runtime_host_vocabulary_boundary ok"
