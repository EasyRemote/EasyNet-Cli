#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CASE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_DIR="$(cd "$CASE_DIR/../.." && pwd)"

WORKSPACE="${1:-$SCRIPT_DIR/demo-workspace}"
DIRECTION="${2:-factorized gap in discrete diffusion LMs}"
TOPIC="${3:-$DIRECTION}"

python3 "$SCRIPT_DIR/aris_native_runtime.py" demo \
  --workspace "$WORKSPACE" \
  --direction "$DIRECTION" \
  --topic "$TOPIC"

if [[ "${COMPILE_EAL:-0}" == "1" ]]; then
  mkdir -p "$WORKSPACE/ir"
  if command -v cargo >/dev/null 2>&1; then
    COMPILE_CMD=(cargo run --manifest-path "$REPO_DIR/Cargo.toml" -- mission compile)
  elif [[ -x "$REPO_DIR/target/debug/easynet" ]]; then
    COMPILE_CMD=("$REPO_DIR/target/debug/easynet" mission compile)
  else
    echo "Skip EAL compile: neither cargo nor target/debug/easynet is available." >&2
    exit 0
  fi

  for mission in \
    w1_idea_discovery.eal \
    w2_auto_review_loop.eal \
    w3_paper_writing_handoff.eal \
    full_research_pipeline.eal
  do
    "${COMPILE_CMD[@]}" "$CASE_DIR/eal/$mission" --emit-ir > "$WORKSPACE/ir/${mission%.eal}.ir.json"
  done
fi
