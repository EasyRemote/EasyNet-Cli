#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/tools/scripts/remoteapp-cross-device-remoteapp-e2e.sh"

fail() {
  printf 'test_remoteapp_cross_device_remoteapp_e2e: %s\n' "$1" >&2
  exit 1
}

"$SCRIPT" --self-test >/dev/null

OUT_DIR="$(mktemp -d)"
trap 'rm -rf "$OUT_DIR"' EXIT

"$SCRIPT" --out-dir "$OUT_DIR/skip" >/tmp/remoteapp-cross-device-remoteapp-skip.out
grep -q '"status": "skipped"' "$OUT_DIR/skip/report.json" || \
  fail "default mode must emit skipped report"
grep -q "SKIP" /tmp/remoteapp-cross-device-remoteapp-skip.out || \
  fail "default output must be explicit skipped evidence"

EASYNET_REMOTEAPP_CROSS_DEVICE_REMOTEAPP_OUT_DIR="$OUT_DIR/good" "$SCRIPT" --self-test >/dev/null
if EASYNET_REMOTEAPP_CROSS_DEVICE_REMOTEAPP_E2E=1 "$SCRIPT" --run \
    --evidence-json "$OUT_DIR/good/evidence.json" --out-dir "$OUT_DIR/self-test-as-live" \
    >/tmp/remoteapp-cross-device-remoteapp-origin.out 2>&1; then
  fail "verifier accepted contract self-test evidence in run mode"
fi
grep -q "evidence_origin must be live_runner" /tmp/remoteapp-cross-device-remoteapp-origin.out || \
  fail "self-test provenance rejection was not explicit"
python3 - "$OUT_DIR/good/evidence.json" <<'PY'
import json
import sys

path = sys.argv[1]
evidence = json.load(open(path, encoding="utf-8"))
evidence["evidence_origin"] = "live_runner"
json.dump(evidence, open(path, "w", encoding="utf-8"), indent=2)
PY
EASYNET_REMOTEAPP_CROSS_DEVICE_REMOTEAPP_E2E=1 "$SCRIPT" --run \
  --evidence-json "$OUT_DIR/good/evidence.json" \
  --out-dir "$OUT_DIR/good-run" >/dev/null

python3 - "$OUT_DIR/good-run/report.json" <<'PY'
import json
import sys

report = json.load(open(sys.argv[1], encoding="utf-8"))
assert report["evidence_origin"] == "live_runner"
scenarios = {scenario["target_kind"]: scenario for scenario in report["scenarios"]}
summary = scenarios["application"]["remoteapp_summary"]
assert "/user/" in summary["caller_ura"]
assert "/agent/" in summary["callee_ura"]
assert summary["provider_device_ura"].startswith("easynet:///")
assert summary["client_endpoint_id"]
assert summary["selected_resource_ura"].startswith("easynet:///")
assert summary["session_id"]
for field in (
    "remote_execution_boundary",
    "remote_target_inventory_seen",
    "abilities_bound",
    "capture_provider_bound",
    "capture_resource_bound",
    "capture_target_kind_bound",
    "capture_remote_target_inventory_seen",
    "media_provider_bound",
    "media_resource_bound",
    "media_session_bound",
    "production_media_pipeline",
    "rendered_after_connected",
    "rendered_on_client_endpoint",
    "client_endpoint_bound",
    "input_policy_checked",
    "input_policy_session_bound",
    "terminal_receipt_visible",
    "terminal_receipt_session_bound",
    "terminal_receipt_subject_bound",
    "end_invocation_receipt_verified",
):
    assert summary[field] is True
assert summary["capture_frames_captured"] > 0
assert summary["capture_counter_source"] in {
    "provider_media_stats.frames_encoded",
    "provider_capture_stats.frames_captured",
}
assert summary["media_transport"] in {"webrtc", "easynet_relay_webrtc"}
assert summary["peer_connection_state"] == "connected"
assert summary["ice_connection_state"] in {"connected", "completed"}
assert summary["selected_candidate_pair_id"]
assert summary["video_codec"].lower() == "h264"
assert summary["media_frames_rendered"] > 0
assert summary["input_policy_mode"] in {"interactive", "view_only", "policy_blocked"}
assert summary["terminal_reason"] in {
    "caller_ended",
    "user_cancelled",
    "cross_device_remoteapp_e2e_cleanup",
}
assert summary["end_invocation_receipt_ura"].startswith("easynet:///")
assert summary["end_invocation_receipt_hash"]
PY

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/device-caller.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
device_caller = "easynet:///r/localhost/device/not-an-admitted-browser-caller"
evidence["topology"]["observed_remote_endpoints"][0]["caller_ura"] = device_caller
for scenario in evidence["scenarios"]:
    scenario["caller_ura"] = device_caller
    for ability in scenario["abilities"]:
        ability["caller_ura"] = device_caller
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if EASYNET_REMOTEAPP_CROSS_DEVICE_REMOTEAPP_E2E=1 "$SCRIPT" --run \
    --evidence-json "$OUT_DIR/device-caller.json" \
    --out-dir "$OUT_DIR/device-caller" \
    >/tmp/remoteapp-cross-device-remoteapp-device-caller.out 2>&1; then
  fail "verifier promoted a Browser peer to a Device caller"
fi
grep -q "caller_ura must identify an admitted principal" \
  /tmp/remoteapp-cross-device-remoteapp-device-caller.out || \
  fail "Device caller rejection was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/missing-window.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["scenarios"] = [
    scenario for scenario in evidence["scenarios"]
    if scenario["target_kind"] != "window"
]
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if EASYNET_REMOTEAPP_CROSS_DEVICE_REMOTEAPP_E2E=1 "$SCRIPT" --run --evidence-json "$OUT_DIR/missing-window.json" --out-dir "$OUT_DIR/missing-window" >/tmp/remoteapp-cross-device-remoteapp-missing-window.out 2>&1; then
  fail "verifier accepted evidence without window target scenario"
fi
grep -q "missing target scenarios: window" /tmp/remoteapp-cross-device-remoteapp-missing-window.out || \
  fail "missing window failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/no-inventory.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["scenarios"][0]["remote_target_inventory_seen"] = False
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if EASYNET_REMOTEAPP_CROSS_DEVICE_REMOTEAPP_E2E=1 "$SCRIPT" --run --evidence-json "$OUT_DIR/no-inventory.json" --out-dir "$OUT_DIR/no-inventory" >/tmp/remoteapp-cross-device-remoteapp-no-inventory.out 2>&1; then
  fail "verifier accepted evidence without remote target inventory"
fi
grep -q "remote_target_inventory_seen must be true" /tmp/remoteapp-cross-device-remoteapp-no-inventory.out || \
  fail "remote target inventory failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/no-input-policy.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["scenarios"][0]["input_policy"]["checked"] = False
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if EASYNET_REMOTEAPP_CROSS_DEVICE_REMOTEAPP_E2E=1 "$SCRIPT" --run --evidence-json "$OUT_DIR/no-input-policy.json" --out-dir "$OUT_DIR/no-input-policy" >/tmp/remoteapp-cross-device-remoteapp-no-input-policy.out 2>&1; then
  fail "verifier accepted evidence without input policy observation"
fi
grep -q "input_policy.checked must be true" /tmp/remoteapp-cross-device-remoteapp-no-input-policy.out || \
  fail "input policy failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/wrong-session.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["scenarios"][0]["media"]["session_id"] = "rd-unrelated-session"
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if EASYNET_REMOTEAPP_CROSS_DEVICE_REMOTEAPP_E2E=1 "$SCRIPT" --run --evidence-json "$OUT_DIR/wrong-session.json" --out-dir "$OUT_DIR/wrong-session" >/tmp/remoteapp-cross-device-remoteapp-wrong-session.out 2>&1; then
  fail "verifier accepted media evidence bound to a different session"
fi
grep -q "media session_id must bind session_id" /tmp/remoteapp-cross-device-remoteapp-wrong-session.out || \
  fail "media session binding failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/diagnostic-attach.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
scenario = evidence["scenarios"][0]
scenario["abilities"] = [
    ability
    for ability in scenario["abilities"]
    if ability["name"] not in {
        "remote_desktop.set_description",
        "remote_desktop.report_client_state",
    }
]
scenario["abilities"].append({
    "name": "remote_desktop.attach",
    "subject_ura": scenario["selected_resource_ura"],
    "session_id": scenario["session_id"],
})
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if EASYNET_REMOTEAPP_CROSS_DEVICE_REMOTEAPP_E2E=1 "$SCRIPT" --run \
    --evidence-json "$OUT_DIR/diagnostic-attach.json" \
    --out-dir "$OUT_DIR/diagnostic-attach" \
    >/tmp/remoteapp-cross-device-remoteapp-diagnostic-attach.out 2>&1; then
  fail "verifier accepted diagnostic attach as the production cross-device media path"
fi
grep -q "diagnostic remote_desktop.attach cannot prove the production WebRTC path" \
  /tmp/remoteapp-cross-device-remoteapp-diagnostic-attach.out || \
  fail "diagnostic attach rejection was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/no-selected-pair.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["scenarios"][0]["media"]["selected_candidate_pair_id"] = ""
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if EASYNET_REMOTEAPP_CROSS_DEVICE_REMOTEAPP_E2E=1 "$SCRIPT" --run \
    --evidence-json "$OUT_DIR/no-selected-pair.json" \
    --out-dir "$OUT_DIR/no-selected-pair" \
    >/tmp/remoteapp-cross-device-remoteapp-no-selected-pair.out 2>&1; then
  fail "verifier accepted cross-device WebRTC evidence without a selected ICE pair"
fi
grep -q "media.selected_candidate_pair_id must be recorded" \
  /tmp/remoteapp-cross-device-remoteapp-no-selected-pair.out || \
  fail "missing selected ICE pair rejection was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/unverified-end-receipt.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["scenarios"][0]["end_invocation_receipt"]["verified"] = False
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if EASYNET_REMOTEAPP_CROSS_DEVICE_REMOTEAPP_E2E=1 "$SCRIPT" --run \
    --evidence-json "$OUT_DIR/unverified-end-receipt.json" \
    --out-dir "$OUT_DIR/unverified-end-receipt" \
    >/tmp/remoteapp-cross-device-remoteapp-unverified-end-receipt.out 2>&1; then
  fail "verifier accepted an unverified end_session Invocation receipt"
fi
grep -q "end_invocation_receipt.verified must be true" \
  /tmp/remoteapp-cross-device-remoteapp-unverified-end-receipt.out || \
  fail "unverified end_session Invocation receipt rejection was not explicit"

echo "test_remoteapp_cross_device_remoteapp_e2e: ok"
