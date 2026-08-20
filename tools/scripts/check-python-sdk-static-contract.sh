#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PYTHON_BIN="${PYTHON:-python3}"
CONTRACT="$ROOT/sdk/conformance/python_sdk_type_contract.py"
source "$ROOT/sdk/conformance/python_toolchain.sh"

export PYTHONPATH="$ROOT/sdk/python:$ROOT/../EasyNet-Axon/sdk/python${PYTHONPATH:+:$PYTHONPATH}"

if [[ "${1:-}" == "--self-test" ]]; then
  "$PYTHON_BIN" -m py_compile "$CONTRACT"
  bash -n "$0"
  echo "python-sdk-static-contract self-test ok"
  exit 0
fi

resolve_sdk_python_toolchain "$ROOT" ruff mypy
PYTHON_BIN="$SDK_CONFORMANCE_PYTHON"

"$PYTHON_BIN" -m ruff check \
  "$ROOT/sdk/python/easynet_sdk" \
  "$ROOT/sdk/python/tests" \
  "$CONTRACT"

"$PYTHON_BIN" -m mypy \
  --strict \
  --python-version 3.12 \
  --config-file "$ROOT/sdk/python/pyproject.toml" \
  "$CONTRACT"

echo "python-sdk-static-contract: OK"
