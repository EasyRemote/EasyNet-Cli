#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/tools/scripts/remoteapp-input-injection-e2e.sh"

fail() {
  printf 'test_remoteapp_input_injection_e2e: %s\n' "$1" >&2
  exit 1
}

"$SCRIPT" --self-test >/dev/null

OUT_DIR="$(mktemp -d)"
trap 'rm -rf "$OUT_DIR"' EXIT

"$SCRIPT" --out-dir "$OUT_DIR/skip" >/tmp/remoteapp-input-injection-skip.out
grep -q '"status": "skipped"' "$OUT_DIR/skip/report.json" || \
  fail "default mode must emit skipped report"
grep -q "skipped" /tmp/remoteapp-input-injection-skip.out || \
  fail "default output must be explicit skipped evidence"

EASYNET_REMOTEAPP_INPUT_INJECTION_OUT_DIR="$OUT_DIR/good" "$SCRIPT" --self-test >/dev/null

python3 - "$OUT_DIR/good/report.json" <<'PY'
import json
import re
import sys

report = json.load(open(sys.argv[1], encoding="utf-8"))
platforms = {platform["platform"]: platform for platform in report["platforms"]}
summary = platforms["macos"]["input_summary"]
assert summary["selected_resource_ura"].startswith("easynet:///")
assert summary["session_id"]
assert summary["permission_granted"] is True
assert summary["consent_scope"] == "input_control"
assert summary["input_scope"] == "display_global"
assert summary["focus_validated"] is True
assert summary["coordinate_mapping_validated"] is True
assert summary["target_geometry_revision"] > 0
assert summary["target_focus_epoch"] > 0
assert summary["source_only_proof"] is False
assert summary["policy_only"] is False
assert summary["stale_client_sequence_rejected"] is True
assert summary["terminal_receipt_visible"] is True
assert summary["terminal_receipt_session_bound"] is True
applied = {entry["kind"]: entry for entry in summary["applied_inputs"]}
assert set(applied) == {"pointer", "keyboard"}
for entry in applied.values():
    assert entry["result"] == "input_applied"
    assert entry["event_type"] == "INPUT_FRAME_APPLIED"
    assert re.fullmatch(r"rdinp1_[0-9a-f]{32}", entry["input_event_id"])
    assert entry["transport_epoch"] > 0
    assert entry["accepted_count"] > 0
    assert entry["latency_ms"] <= summary["latency_threshold_ms"]
    assert entry["os_effect_observed"] is True
    assert entry["observer_independent_from_injector"] is True
    assert entry["os_effect_bound"] is True
    assert entry["target_geometry_revision_bound"] is True
    assert entry["target_focus_epoch_bound"] is True
assert applied["pointer"]["coordinate_mapping"] == "target_geometry_revision_matched"
assert applied["pointer"]["within_tolerance_px"] is True
assert applied["keyboard"]["focused_resource_bound"] is True
assert applied["keyboard"]["key_code_matched"] is True
PY

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/missing-keyboard.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["platforms"][0]["input_results"] = [
    item for item in evidence["platforms"][0]["input_results"]
    if item["kind"] != "keyboard"
]
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/missing-keyboard.json" --out-dir "$OUT_DIR/missing-keyboard" >/tmp/remoteapp-input-injection-missing-keyboard.out 2>&1; then
  fail "verifier accepted input evidence without keyboard result"
fi
grep -q "missing input results: keyboard" /tmp/remoteapp-input-injection-missing-keyboard.out || \
  fail "missing keyboard failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/high-latency.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["platforms"][0]["input_results"][0]["latency_ms"] = 400
evidence["platforms"][0]["latency_summary"]["max_ms"] = 400
evidence["platforms"][0]["latency_summary"]["p95_ms"] = 400
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/high-latency.json" --out-dir "$OUT_DIR/high-latency" >/tmp/remoteapp-input-injection-high-latency.out 2>&1; then
  fail "verifier accepted high-latency input evidence"
fi
grep -q "latency_ms must be within threshold" /tmp/remoteapp-input-injection-high-latency.out || \
  fail "latency failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/no-permission.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["platforms"][0]["permission"]["accessibility_granted"] = False
evidence["platforms"][0]["permission"]["input_injection_granted"] = False
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/no-permission.json" --out-dir "$OUT_DIR/no-permission" >/tmp/remoteapp-input-injection-no-permission.out 2>&1; then
  fail "verifier accepted input evidence without OS input permission"
fi
grep -q "OS input permission must be granted" /tmp/remoteapp-input-injection-no-permission.out || \
  fail "permission failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/no-input-event-id.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
del evidence["platforms"][0]["input_results"][0]["input_event_id"]
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/no-input-event-id.json" --out-dir "$OUT_DIR/no-input-event-id" >/tmp/remoteapp-input-injection-no-event-id.out 2>&1; then
  fail "verifier accepted input evidence without stable input_event_id"
fi
grep -q "input_event_id must be recorded" /tmp/remoteapp-input-injection-no-event-id.out || \
  fail "input_event_id failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/fake-input-event-id.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["platforms"][0]["input_results"][0]["input_event_id"] = "input-event-pointer-1"
evidence["platforms"][0]["input_results"][0]["os_effect"]["input_event_id"] = "input-event-pointer-1"
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/fake-input-event-id.json" --out-dir "$OUT_DIR/fake-input-event-id" >/tmp/remoteapp-input-injection-fake-event-id.out 2>&1; then
  fail "verifier accepted input evidence with non-daemon input_event_id"
fi
grep -q "input_event_id must be daemon-applied" /tmp/remoteapp-input-injection-fake-event-id.out || \
  fail "non-daemon input_event_id failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/no-transport-epoch.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
del evidence["platforms"][0]["input_results"][0]["transport_epoch"]
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/no-transport-epoch.json" --out-dir "$OUT_DIR/no-transport-epoch" >/tmp/remoteapp-input-injection-no-transport-epoch.out 2>&1; then
  fail "verifier accepted input evidence without transport_epoch"
fi
grep -q "transport_epoch must be positive" /tmp/remoteapp-input-injection-no-transport-epoch.out || \
  fail "transport_epoch failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/injector-observer.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["platforms"][0]["input_results"][0]["os_effect"]["observer_independent_from_injector"] = False
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/injector-observer.json" --out-dir "$OUT_DIR/injector-observer" >/tmp/remoteapp-input-injection-injector-observer.out 2>&1; then
  fail "verifier accepted OS effect observed by the injection path"
fi
grep -q "os_effect observer must be independent from injector" /tmp/remoteapp-input-injection-injector-observer.out || \
  fail "observer independence failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/wrong-effect-event-id.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["platforms"][0]["input_results"][0]["os_effect"]["input_event_id"] = "unrelated-input-event"
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/wrong-effect-event-id.json" --out-dir "$OUT_DIR/wrong-effect-event-id" >/tmp/remoteapp-input-injection-wrong-effect-event-id.out 2>&1; then
  fail "verifier accepted OS effect bound to a different input event"
fi
grep -q "os_effect input_event_id must bind input_event_id" /tmp/remoteapp-input-injection-wrong-effect-event-id.out || \
  fail "OS effect input_event_id binding failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/wrong-focus-epoch.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["platforms"][0]["input_results"][0]["os_effect"]["target_focus_epoch"] = 999
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/wrong-focus-epoch.json" --out-dir "$OUT_DIR/wrong-focus-epoch" >/tmp/remoteapp-input-injection-wrong-focus-epoch.out 2>&1; then
  fail "verifier accepted OS effect bound to a stale focus epoch"
fi
grep -q "os_effect target_focus_epoch must match platform scenario" /tmp/remoteapp-input-injection-wrong-focus-epoch.out || \
  fail "OS effect focus-epoch binding failure was not explicit"

echo "test_remoteapp_input_injection_e2e: ok"
