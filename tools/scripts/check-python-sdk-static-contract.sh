#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PYTHON_BIN="${PYTHON:-python}"
RUFF_BIN="${RUFF:-ruff}"
MYPY_BIN="${MYPY:-mypy}"
CONTRACT="$ROOT/sdk/conformance/python_sdk_type_contract.py"

export PYTHONPATH="$ROOT/sdk/python:$ROOT/../EasyNet-Axon/sdk/python${PYTHONPATH:+:$PYTHONPATH}"

if [[ "${1:-}" == "--self-test" ]]; then
  "$PYTHON_BIN" -m py_compile "$CONTRACT"
  bash -n "$0"
  echo "python-sdk-static-contract self-test ok"
  exit 0
fi

command -v "$RUFF_BIN" >/dev/null 2>&1 || {
  echo "python-sdk-static-contract: ruff tool not found: $RUFF_BIN" >&2
  exit 1
}
command -v "$MYPY_BIN" >/dev/null 2>&1 || {
  echo "python-sdk-static-contract: mypy tool not found: $MYPY_BIN" >&2
  exit 1
}

"$RUFF_BIN" check \
  "$ROOT/sdk/python/easynet_sdk" \
  "$ROOT/sdk/python/tests" \
  "$CONTRACT"

"$MYPY_BIN" \
  --strict \
  --python-version 3.12 \
  --config-file "$ROOT/sdk/python/pyproject.toml" \
  "$CONTRACT"

echo "python-sdk-static-contract: OK"
