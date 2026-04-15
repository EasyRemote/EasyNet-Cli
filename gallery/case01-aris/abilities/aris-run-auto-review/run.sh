#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CASE_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
WORKSPACE="${ARIS_WORKSPACE:-$CASE_DIR/runtime/demo-workspace}"
python3 "$CASE_DIR/runtime/aris_native_runtime.py" ability aris_run_auto_review \
  --workspace "$WORKSPACE" \
  --topic "${ARIS_TOPIC:-factorized gap in discrete diffusion LMs}" \
  --difficulty "${ARIS_DIFFICULTY:-medium}" \
  --rounds "${ARIS_ROUNDS:-2}"
