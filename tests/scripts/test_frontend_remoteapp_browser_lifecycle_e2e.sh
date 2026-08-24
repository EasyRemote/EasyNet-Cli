#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh"

fail() {
  printf 'test_frontend_remoteapp_browser_lifecycle_e2e: %s\n' "$1" >&2
  exit 1
}

"$SCRIPT" --self-test >/dev/null

OUT_DIR="$(mktemp -d)"
trap 'rm -rf "$OUT_DIR"' EXIT

"$SCRIPT" --out-dir "$OUT_DIR/skip" >/tmp/frontend-remoteapp-browser-lifecycle-skip.out
grep -q '"status": "skipped"' "$OUT_DIR/skip/report.json" || \
  fail "default mode must emit skipped report"
grep -q "skipped" /tmp/frontend-remoteapp-browser-lifecycle-skip.out || \
  fail "default output must be explicit skipped evidence"

EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_OUT_DIR="$OUT_DIR/good" "$SCRIPT" --self-test >/dev/null

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/input-applied.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
session_id = evidence["session_id"]
for step in evidence["steps"]:
    if step["name"] != "input_control_attempted_or_policy_blocked":
        continue
    step.clear()
    step.update({
        "name": "input_control_attempted_or_policy_blocked",
        "status": "passed",
        "evidence_source": "browser_automation",
        "component_snapshot_only": False,
        "observed_at_ms": 1787332000110,
        "result": "input_applied",
        "visible_status": "input interactive ready",
        "client_sequence": 7,
        "target_focus_epoch": 11,
        "target_geometry_revision": 23,
        "latency_ms": 17,
        "submitted_frame": {
            "type": "key",
            "action": "down",
            "code": "KeyA",
            "client_sequence": 7,
            "sent_at_ms": 1787332000100,
            "target_focus_epoch": 11,
            "target_geometry_revision": 23,
        },
        "applied_event": {
            "event_type": "INPUT_FRAME_APPLIED",
            "session_id": session_id,
            "client_sequence": 7,
            "target_focus_epoch": 11,
            "target_geometry_revision": 23,
        },
    })
for step in evidence["steps"]:
    if step["name"] != "input_control_after_resume":
        continue
    step.clear()
    step.update({
        "name": "input_control_after_resume",
        "status": "passed",
        "evidence_source": "browser_automation",
        "component_snapshot_only": False,
        "observed_at_ms": 1787332000170,
        "result": "input_applied",
        "visible_status": "input interactive ready",
        "client_sequence": 8,
        "target_focus_epoch": 12,
        "target_geometry_revision": 24,
        "latency_ms": 18,
        "submitted_frame": {
            "type": "key",
            "action": "down",
            "code": "KeyB",
            "client_sequence": 8,
            "sent_at_ms": 1787332000160,
            "target_focus_epoch": 12,
            "target_geometry_revision": 24,
        },
        "applied_event": {
            "event_type": "INPUT_FRAME_APPLIED",
            "session_id": session_id,
            "client_sequence": 8,
            "target_focus_epoch": 12,
            "target_geometry_revision": 24,
        },
    })
evidence["transport_resume"]["input_result_before"] = "input_applied"
evidence["transport_resume"]["input_result_after"] = "input_applied"
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
"$SCRIPT" --run --evidence-json "$OUT_DIR/input-applied.json" --out-dir "$OUT_DIR/input-applied" >/dev/null
grep -q '"status": "passed"' "$OUT_DIR/input-applied/report.json" || \
  fail "input_applied evidence with focus epoch must pass"

python3 - "$OUT_DIR/input-applied.json" "$OUT_DIR/stale-resume-input.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
for step in evidence["steps"]:
    if step["name"] == "input_control_after_resume":
        step["client_sequence"] = 7
        step["submitted_frame"]["client_sequence"] = 7
        step["applied_event"]["client_sequence"] = 7
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/stale-resume-input.json" --out-dir "$OUT_DIR/stale-resume-input" >/tmp/frontend-remoteapp-browser-lifecycle-stale-resume-input.out 2>&1; then
  fail "verifier accepted post-resume input without sequence advancement"
fi
grep -q "post-resume input client_sequence must advance" /tmp/frontend-remoteapp-browser-lifecycle-stale-resume-input.out || \
  fail "stale post-resume input failure was not explicit"

python3 - "$OUT_DIR/input-applied.json" "$OUT_DIR/missing-submitted-focus.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
for step in evidence["steps"]:
    if step["name"] == "input_control_attempted_or_policy_blocked":
        del step["submitted_frame"]["target_focus_epoch"]
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/missing-submitted-focus.json" --out-dir "$OUT_DIR/missing-submitted-focus" >/tmp/frontend-remoteapp-browser-lifecycle-missing-submitted-focus.out 2>&1; then
  fail "verifier accepted input_applied evidence without submitted_frame target_focus_epoch"
fi
grep -q "submitted_frame target_focus_epoch must match input_applied target_focus_epoch" /tmp/frontend-remoteapp-browser-lifecycle-missing-submitted-focus.out || \
  fail "missing submitted target_focus_epoch failure was not explicit"

python3 - "$OUT_DIR/input-applied.json" "$OUT_DIR/wrong-applied-focus.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
for step in evidence["steps"]:
    if step["name"] == "input_control_attempted_or_policy_blocked":
        step["applied_event"]["target_focus_epoch"] = 12
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/wrong-applied-focus.json" --out-dir "$OUT_DIR/wrong-applied-focus" >/tmp/frontend-remoteapp-browser-lifecycle-wrong-applied-focus.out 2>&1; then
  fail "verifier accepted input_applied evidence with stale applied_event target_focus_epoch"
fi
grep -q "applied_event target_focus_epoch must match input_applied target_focus_epoch" /tmp/frontend-remoteapp-browser-lifecycle-wrong-applied-focus.out || \
  fail "stale applied target_focus_epoch failure was not explicit"

echo "test_frontend_remoteapp_browser_lifecycle_e2e: ok"
