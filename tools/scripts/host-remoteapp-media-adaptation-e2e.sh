#!/usr/bin/env bash
# Real Browser RemoteApp media adaptation matrix runner.
#
# Runs the production frontend lifecycle against one selected Resource for the
# baseline, degraded-network, and receiver-backpressure scenarios. Scenario
# commands are executed by the browser runner only after decoded media is
# visible. Every scenario has an explicit prepare/reset boundary, and reset is
# retried from the process trap so a failed browser run cannot leave host
# impairment behind.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
DEFAULT_EASYNET_ROOT="$(cd "$REPO_ROOT/.." && pwd)/EasyNet"

EASYNET_ROOT="${EASYNET_REMOTEAPP_EASYNET_REPO_ROOT:-$DEFAULT_EASYNET_ROOT}"
FRONTEND_URL="${EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_FRONTEND_URL:-}"
DEVICE_ID="${EASYNET_REMOTEAPP_BROWSER_DEVICE_ID:-}"
OUT_DIR="${EASYNET_REMOTEAPP_MEDIA_MATRIX_OUT_DIR:-$REPO_ROOT/target/e2e/host-remoteapp-media-adaptation/$(date -u +%Y%m%d-%H%M%S)-$$}"
EVIDENCE_JSON="${EASYNET_REMOTEAPP_MEDIA_ADAPTATION_EVIDENCE_JSON:-}"
BASELINE_PREPARE="${EASYNET_REMOTEAPP_MEDIA_BASELINE_PREPARE_COMMAND:-}"
BASELINE_RESET="${EASYNET_REMOTEAPP_MEDIA_BASELINE_RESET_COMMAND:-}"
DEGRADED_APPLY="${EASYNET_REMOTEAPP_MEDIA_DEGRADED_NETWORK_APPLY_COMMAND:-}"
DEGRADED_RESET="${EASYNET_REMOTEAPP_MEDIA_DEGRADED_NETWORK_RESET_COMMAND:-}"
BACKPRESSURE_APPLY="${EASYNET_REMOTEAPP_MEDIA_BACKPRESSURE_APPLY_COMMAND:-}"
BACKPRESSURE_RESET="${EASYNET_REMOTEAPP_MEDIA_BACKPRESSURE_RESET_COMMAND:-}"
LIFECYCLE_RUNNER_CMD="${EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_RUNNER_CMD:-}"
AGGREGATOR="${EASYNET_REMOTEAPP_MEDIA_AGGREGATOR:-$SELF_DIR/aggregate-remoteapp-media-adaptation-evidence.py}"
ACTIVE_RESET=""
ACTIVE_SCENARIO=""

usage() {
  cat <<'USAGE'
Usage:
  host-remoteapp-media-adaptation-e2e.sh [options]

Options:
  --frontend-url URL    Real EasyNet frontend URL.
  --device-id ID        Paired provider device id shown by the frontend.
  --out-dir DIR         Artifact directory.
  --evidence-json PATH  Aggregated matrix output. Defaults to
                        EASYNET_REMOTEAPP_MEDIA_ADAPTATION_EVIDENCE_JSON or
                        <out-dir>/evidence.json.
  -h, --help            Show this help.

Required environment:
  EASYNET_REMOTEAPP_MEDIA_BASELINE_PREPARE_COMMAND
  EASYNET_REMOTEAPP_MEDIA_BASELINE_RESET_COMMAND
  EASYNET_REMOTEAPP_MEDIA_DEGRADED_NETWORK_APPLY_COMMAND
  EASYNET_REMOTEAPP_MEDIA_DEGRADED_NETWORK_RESET_COMMAND
  EASYNET_REMOTEAPP_MEDIA_BACKPRESSURE_APPLY_COMMAND
  EASYNET_REMOTEAPP_MEDIA_BACKPRESSURE_RESET_COMMAND

The browser lifecycle runner also consumes its normal authentication, target,
Chrome, and timing variables. The selected Resource URA and media pipeline must
remain identical across all three runs; the canonical aggregator rejects drift.

The apply/reset commands are operational inputs and are never copied into the
evidence artifact. Only SHA-256 command fingerprints and execution outcomes are
recorded, so credentials or host topology embedded in a command are not leaked.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --frontend-url) FRONTEND_URL="${2:?missing value for --frontend-url}"; shift 2 ;;
    --device-id) DEVICE_ID="${2:?missing value for --device-id}"; shift 2 ;;
    --out-dir) OUT_DIR="${2:?missing value for --out-dir}"; shift 2 ;;
    --evidence-json) EVIDENCE_JSON="${2:?missing value for --evidence-json}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

die() {
  echo "[FAIL] $*" >&2
  exit 1
}

require_value() {
  local name="$1"
  local value="$2"
  [[ -n "$value" ]] || die "$name is required"
}

command_fingerprint() {
  python3 - "$1" <<'PY'
import hashlib
import sys
print("sha256:" + hashlib.sha256(sys.argv[1].encode("utf-8")).hexdigest())
PY
}

run_fixture_command() {
  local scenario="$1"
  local phase="$2"
  local command="$3"
  mkdir -p "$OUT_DIR/$scenario"
  # Fixture commands may contain deployment credentials or print network
  # topology. Their command text and output are deliberately excluded from
  # artifacts; the redacted plan records only a command fingerprint.
  if ! bash -lc "$command" >/dev/null 2>&1; then
    die "$scenario $phase command failed"
  fi
}

reset_active_fixture() {
  local exit_code=$?
  if [[ -n "$ACTIVE_RESET" ]]; then
    local reset_command="$ACTIVE_RESET"
    local reset_scenario="$ACTIVE_SCENARIO"
    ACTIVE_RESET=""
    ACTIVE_SCENARIO=""
    if ! run_fixture_command "$reset_scenario" reset "$reset_command"; then
      exit_code=1
    fi
  fi
  return "$exit_code"
}

trap reset_active_fixture EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

require_value "frontend URL" "$FRONTEND_URL"
require_value "device id" "$DEVICE_ID"
require_value "EASYNET_REMOTEAPP_MEDIA_BASELINE_PREPARE_COMMAND" "$BASELINE_PREPARE"
require_value "EASYNET_REMOTEAPP_MEDIA_BASELINE_RESET_COMMAND" "$BASELINE_RESET"
require_value "EASYNET_REMOTEAPP_MEDIA_DEGRADED_NETWORK_APPLY_COMMAND" "$DEGRADED_APPLY"
require_value "EASYNET_REMOTEAPP_MEDIA_DEGRADED_NETWORK_RESET_COMMAND" "$DEGRADED_RESET"
require_value "EASYNET_REMOTEAPP_MEDIA_BACKPRESSURE_APPLY_COMMAND" "$BACKPRESSURE_APPLY"
require_value "EASYNET_REMOTEAPP_MEDIA_BACKPRESSURE_RESET_COMMAND" "$BACKPRESSURE_RESET"
command -v bash >/dev/null 2>&1 || die "bash is required"
command -v python3 >/dev/null 2>&1 || die "python3 is required"

if [[ -z "$LIFECYCLE_RUNNER_CMD" ]]; then
  LIFECYCLE_RUNNER="$EASYNET_ROOT/Frontend/scripts/remoteapp-browser-lifecycle.mjs"
  [[ -f "$LIFECYCLE_RUNNER" ]] || die "missing Browser lifecycle runner: $LIFECYCLE_RUNNER"
  command -v node >/dev/null 2>&1 || die "node is required"
  printf -v LIFECYCLE_RUNNER_CMD 'node %q' "$LIFECYCLE_RUNNER"
fi
[[ -f "$AGGREGATOR" ]] || die "missing media evidence aggregator: $AGGREGATOR"

mkdir -p "$OUT_DIR"
if [[ -z "$EVIDENCE_JSON" ]]; then
  EVIDENCE_JSON="$OUT_DIR/evidence.json"
fi
mkdir -p "$(dirname "$EVIDENCE_JSON")"

FIXTURE_PLAN_JSON="$OUT_DIR/fixture-plan.json"
python3 - "$FIXTURE_PLAN_JSON" \
  "$(command_fingerprint "$BASELINE_PREPARE")" \
  "$(command_fingerprint "$BASELINE_RESET")" \
  "$(command_fingerprint "$DEGRADED_APPLY")" \
  "$(command_fingerprint "$DEGRADED_RESET")" \
  "$(command_fingerprint "$BACKPRESSURE_APPLY")" \
  "$(command_fingerprint "$BACKPRESSURE_RESET")" <<'PY'
import json
import pathlib
import sys

output, baseline_prepare, baseline_reset, degraded_apply, degraded_reset, pressure_apply, pressure_reset = sys.argv[1:]
pathlib.Path(output).write_text(json.dumps({
    "schema_version": 1,
    "commands_redacted": True,
    "baseline": {
        "prepare_command_hash": baseline_prepare,
        "reset_command_hash": baseline_reset,
    },
    "degraded_network": {
        "apply_command_hash": degraded_apply,
        "reset_command_hash": degraded_reset,
    },
    "backpressure": {
        "apply_command_hash": pressure_apply,
        "reset_command_hash": pressure_reset,
    },
}, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY

run_browser_scenario() {
  local scenario="$1"
  local prepare_command="$2"
  local impairment_command="$3"
  local reset_command="$4"
  local scenario_dir="$OUT_DIR/$scenario"
  local scenario_evidence="$scenario_dir/browser-evidence.json"
  mkdir -p "$scenario_dir"

  run_fixture_command "$scenario" prepare "$prepare_command"
  ACTIVE_RESET="$reset_command"
  ACTIVE_SCENARIO="$scenario"

  if ! EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_EVIDENCE_JSON="$scenario_evidence" \
    EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_FRONTEND_URL="$FRONTEND_URL" \
    EASYNET_REMOTEAPP_BROWSER_DEVICE_ID="$DEVICE_ID" \
    EASYNET_REMOTEAPP_BROWSER_MEDIA_SCENARIO="$scenario" \
    EASYNET_REMOTEAPP_BROWSER_IMPAIRMENT_COMMAND="$impairment_command" \
    bash -lc "$LIFECYCLE_RUNNER_CMD" \
      >"$scenario_dir/browser.stdout.txt" \
      2>"$scenario_dir/browser.stderr.txt"; then
    die "$scenario Browser lifecycle failed; see $scenario_dir/browser.stderr.txt"
  fi
  [[ -s "$scenario_evidence" ]] || die "$scenario Browser lifecycle did not write evidence"

  run_fixture_command "$scenario" reset "$reset_command"
  ACTIVE_RESET=""
  ACTIVE_SCENARIO=""
}

# A clean baseline is established and reset explicitly, so later failed
# scenarios cannot inherit baseline host state.
run_browser_scenario baseline "$BASELINE_PREPARE" "" "$BASELINE_RESET"
run_browser_scenario degraded_network "$DEGRADED_RESET" "$DEGRADED_APPLY" "$DEGRADED_RESET"
run_browser_scenario backpressure "$BACKPRESSURE_RESET" "$BACKPRESSURE_APPLY" "$BACKPRESSURE_RESET"

python3 "$AGGREGATOR" \
  --baseline "$OUT_DIR/baseline/browser-evidence.json" \
  --degraded-network "$OUT_DIR/degraded_network/browser-evidence.json" \
  --backpressure "$OUT_DIR/backpressure/browser-evidence.json" \
  --output "$EVIDENCE_JSON"

[[ -s "$EVIDENCE_JSON" ]] || die "media aggregator did not write $EVIDENCE_JSON"
echo "host RemoteApp media adaptation matrix written: $EVIDENCE_JSON"
