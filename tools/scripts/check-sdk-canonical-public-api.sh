#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DEFAULT_MANIFEST="$ROOT/sdk/conformance/canonical-public-api.json"
DEFAULT_MATRIX="$ROOT/sdk/conformance/sdk-parity-matrix.json"
MANIFEST="${SDK_CONCEPT_MANIFEST:-$DEFAULT_MANIFEST}"
VALIDATOR="$ROOT/sdk/conformance/sdk_concepts.py"
PYTHON_BIN="${PYTHON:-python}"
GO_BIN="${GO:-go}"

export PYTHONPATH="$ROOT/sdk/python:$ROOT/../EasyNet-Axon/sdk/python${PYTHONPATH:+:$PYTHONPATH}"

check_generated_outputs() {
  local manifest="$1"
  local matrix="$2"
  local tmp="$3"
  local generated_manifest="$tmp/canonical-public-api.generated.json"
  local generated_matrix="$tmp/sdk-parity-matrix.generated.json"

  "$PYTHON_BIN" "$ROOT/sdk/conformance/rebuild_public_api_model.py" >"$generated_manifest"
  "$PYTHON_BIN" "$ROOT/sdk/conformance/sdk_matrix.py" --generate >"$generated_matrix"
  if ! cmp -s "$generated_manifest" "$manifest"; then
    echo "canonical-public-api: stale generated manifest: sdk/conformance/canonical-public-api.json" >&2
    echo "canonical-public-api: run python3 sdk/conformance/rebuild_public_api_model.py --write" >&2
    return 1
  fi
  if ! cmp -s "$generated_matrix" "$matrix"; then
    echo "canonical-public-api: stale generated matrix: sdk/conformance/sdk-parity-matrix.json" >&2
    echo "canonical-public-api: run python3 sdk/conformance/rebuild_public_api_model.py --write" >&2
    return 1
  fi
}

if [[ "${1:-}" == "--self-test" ]]; then
  mkdir -p "$ROOT/target"
  tmp="$(mktemp -d "$ROOT/target/sdk-concepts-self-test.XXXXXX")"
  trap 'rm -rf "$tmp"' EXIT
  "$PYTHON_BIN" "$ROOT/sdk/conformance/public_api_inventory.py" --self-test
  "$PYTHON_BIN" "$VALIDATOR" --self-test --tmp "$tmp"
  check_generated_outputs "$DEFAULT_MANIFEST" "$DEFAULT_MATRIX" "$tmp"
  cp "$DEFAULT_MANIFEST" "$tmp/stale-public.json"
  "$PYTHON_BIN" - "$tmp/stale-public.json" <<'PY'
import json, sys
from pathlib import Path
path = Path(sys.argv[1])
data = json.loads(path.read_text())
data["schema_version"] = -1
path.write_text(json.dumps(data, indent=2) + "\n")
PY
  if check_generated_outputs "$tmp/stale-public.json" "$DEFAULT_MATRIX" "$tmp" >/dev/null 2>"$tmp/stale.out"; then
    echo "self-test expected stale generated public API manifest to fail" >&2
    exit 1
  fi
  grep -Fq "stale generated manifest" "$tmp/stale.out"
  echo "check-sdk-canonical-public-api self-test ok"
  exit 0
fi

command -v "$GO_BIN" >/dev/null 2>&1 || {
  echo "canonical-public-api: go tool not found: $GO_BIN" >&2
  exit 1
}

"$PYTHON_BIN" "$VALIDATOR" --validate-actual --manifest "$MANIFEST" --go-bin "$GO_BIN"
if [[ "$MANIFEST" == "$DEFAULT_MANIFEST" ]]; then
  tmp="$(mktemp -d "$ROOT/target/sdk-canonical-public-api.XXXXXX")"
  trap 'rm -rf "$tmp"' EXIT
  check_generated_outputs "$DEFAULT_MANIFEST" "$DEFAULT_MATRIX" "$tmp"
fi
echo "canonical-public-api: OK"
