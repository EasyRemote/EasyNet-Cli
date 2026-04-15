#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CASE_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
WORKSPACE="${ARIS_WORKSPACE:-$CASE_DIR/runtime/demo-workspace}"
python3 "$CASE_DIR/runtime/aris_native_runtime.py" ability aris_run_idea_discovery \
  --workspace "$WORKSPACE" \
  --direction "${ARIS_DIRECTION:-factorized gap in discrete diffusion LMs}"
