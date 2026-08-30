#!/usr/bin/env bash

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
RUNNER="$REPO_ROOT/tools/scripts/host-remoteapp-target-monitor-worker-recovery-e2e.sh"
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/remoteapp-worker-recovery-test.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT

BASE_OUT="$TMP_DIR/base"
"$RUNNER" --self-test --out-dir "$BASE_OUT" >/dev/null
BASE_EVIDENCE="$BASE_OUT/evidence.json"

verify_rejected() {
  local fixture="$1"
  local out_dir="$2"
  if "$RUNNER" --verify-self-test-evidence "$fixture" --out-dir "$out_dir" >/dev/null 2>&1; then
    echo "invalid worker-recovery evidence was accepted: $fixture" >&2
    exit 1
  fi
}

python3 - "$BASE_EVIDENCE" "$TMP_DIR/pid.json" "$TMP_DIR/generation.json" \
  "$TMP_DIR/epoch.json" "$TMP_DIR/terminal.json" <<'PY'
import copy
import json
import pathlib
import sys

source = json.load(open(sys.argv[1], encoding="utf-8"))
mutations = []
pid = copy.deepcopy(source)
pid["process"]["after_pid"] += 1
mutations.append(pid)
generation = copy.deepcopy(source)
generation["public_events"][1]["payload"]["restarted_generation"] = 7
mutations.append(generation)
epoch = copy.deepcopy(source)
epoch["browser_worker_recovery"]["media_source_epoch_after"] += 1
mutations.append(epoch)
terminal = copy.deepcopy(source)
terminal["terminal_cleanup"]["terminal"] = False
mutations.append(terminal)
for path, value in zip(sys.argv[2:], mutations):
    pathlib.Path(path).write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY

verify_rejected "$TMP_DIR/pid.json" "$TMP_DIR/reject-pid"
verify_rejected "$TMP_DIR/generation.json" "$TMP_DIR/reject-generation"
verify_rejected "$TMP_DIR/epoch.json" "$TMP_DIR/reject-epoch"
verify_rejected "$TMP_DIR/terminal.json" "$TMP_DIR/reject-terminal"

ARM_PATH="$TMP_DIR/arm/target-monitor-arm.json"
MARKER_PATH="$TMP_DIR/marker/target-monitor-marker.json"
EASYNET_REMOTEAPP_E2E_SESSION_ID=rdp-arm-contract \
EASYNET_REMOTEAPP_E2E_RESOURCE_URA=easynet:///r/localhost/resource/device.dev/streams/window.arm \
  "$RUNNER" --fixture-arm "$ARM_PATH" "$MARKER_PATH"
[[ "$(stat -f '%Lp' "$ARM_PATH")" == 600 ]]
jq -e '
  .schema_version == 1
  and .fault == "crash_target_monitor_generation"
  and .session_id == "rdp-arm-contract"
  and (.nonce | type == "string" and length == 32)
  and (.armed_at_ms | type == "number" and . > 0)
' "$ARM_PATH" >/dev/null

echo "[test_host_remoteapp_target_monitor_worker_recovery_e2e] PASS"
