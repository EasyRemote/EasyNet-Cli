#!/usr/bin/env bash
set -euo pipefail

ROOT="${SDK_RUNTIME_HOST_VOCABULARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
  echo "sdk-runtime-host-vocabulary-boundary: $*" >&2
  exit 1
}

sources=(
  "sdk/go/addressing.go"
  "sdk/go/authorized_runtime_session.go"
  "sdk/go/authority.go"
  "sdk/go/bidi.go"
  "sdk/go/client.go"
  "sdk/go/direct_runtime.go"
  "sdk/go/directory.go"
  "sdk/go/native_runtime_cabi.go"
  "sdk/go/runtime.go"
  "sdk/python/easynet_sdk/client.py"
  "sdk/python/easynet_sdk/transport.py"
)

existing=()
for source in "${sources[@]}"; do
  [[ -f "$source" ]] || fail "missing source: $source"
  existing+=("$source")
done

if rg -n -i -P '\bdaemon\b(?!\s*=)' "${existing[@]}"; then
  fail "generic SDK runtime-host sources preserve daemon vocabulary"
fi

if rg -n -i 'daemon (open|close|start|stop|attach|discover|transport|policy|registry|projection|metadata|control|lifecycle|endpoint)' \
  sdk/go sdk/python/easynet_sdk \
  -g '!**/*test*' \
  -g '!sdk/go/provider/**' \
  -g '!sdk/python/easynet_sdk/providers/**' \
  -g '!sdk/python/easynet_sdk/_axon_pb/**' \
  -g '!*.pb.go'; then
  fail "generic SDK production sources preserve daemon lifecycle/provider vocabulary"
fi

echo "sdk-runtime-host-vocabulary-boundary: OK"
