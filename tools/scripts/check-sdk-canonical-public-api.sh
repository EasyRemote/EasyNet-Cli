#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MANIFEST="${SDK_CONCEPT_MANIFEST:-$ROOT/sdk/conformance/canonical-public-api.json}"
VALIDATOR="$ROOT/sdk/conformance/sdk_concepts.py"
PYTHON_BIN="${PYTHON:-python}"
GO_BIN="${GO:-go}"

export PYTHONPATH="$ROOT/sdk/python:$ROOT/../EasyNet-Axon/sdk/python${PYTHONPATH:+:$PYTHONPATH}"

if [[ "${1:-}" == "--self-test" ]]; then
  mkdir -p "$ROOT/target"
  tmp="$(mktemp -d "$ROOT/target/sdk-concepts-self-test.XXXXXX")"
  trap 'rm -rf "$tmp"' EXIT
  "$PYTHON_BIN" "$ROOT/sdk/conformance/public_api_inventory.py" --self-test
  "$PYTHON_BIN" "$VALIDATOR" --self-test --tmp "$tmp"
  echo "check-sdk-canonical-public-api self-test ok"
  exit 0
fi

command -v "$GO_BIN" >/dev/null 2>&1 || {
  echo "canonical-public-api: go tool not found: $GO_BIN" >&2
  exit 1
}

"$PYTHON_BIN" "$VALIDATOR" --validate-actual --manifest "$MANIFEST" --go-bin "$GO_BIN"
echo "canonical-public-api: OK"
