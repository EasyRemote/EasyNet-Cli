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
EASYNET_REMOTEAPP_CROSS_DEVICE_REMOTEAPP_E2E=1 "$SCRIPT" --run \
  --evidence-json "$OUT_DIR/good/evidence.json" \
  --out-dir "$OUT_DIR/good-run" >/dev/null

python3 - "$OUT_DIR/good-run/report.json" <<'PY'
import json
import sys

report = json.load(open(sys.argv[1], encoding="utf-8"))
scenarios = {scenario["target_kind"]: scenario for scenario in report["scenarios"]}
summary = scenarios["application"]["remoteapp_summary"]
assert summary["caller_device_ura"].startswith("easynet:///")
assert summary["provider_device_ura"].startswith("easynet:///")
assert summary["caller_device_ura"] != summary["provider_device_ura"]
assert summary["selected_resource_ura"].startswith("easynet:///")
assert summary["session_id"]
for field in (
    "distinct_devices",
    "remote_target_inventory_seen",
    "abilities_bound",
    "capture_provider_bound",
    "capture_resource_bound",
    "capture_target_kind_bound",
    "capture_remote_target_inventory_seen",
    "media_provider_bound",
    "media_resource_bound",
    "media_session_bound",
    "rendered_on_caller_device",
    "input_policy_checked",
    "input_policy_session_bound",
    "terminal_receipt_visible",
    "terminal_receipt_session_bound",
):
    assert summary[field] is True
assert summary["capture_frames_captured"] > 0
assert summary["media_transport"] in {"webrtc", "easynet_relay_webrtc"}
assert summary["media_frames_rendered"] > 0
assert summary["input_policy_mode"] in {"interactive", "view_only", "policy_blocked"}
assert summary["terminal_reason"] in {
    "caller_ended",
    "user_cancelled",
    "cross_device_remoteapp_e2e_cleanup",
}
PY

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

echo "test_remoteapp_cross_device_remoteapp_e2e: ok"
