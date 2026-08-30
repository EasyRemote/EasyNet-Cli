#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/tools/scripts/remoteapp-multi-window-tracking-e2e.sh"

fail() {
  printf 'test_remoteapp_multi_window_tracking_e2e: %s\n' "$1" >&2
  exit 1
}

"$SCRIPT" --self-test >/dev/null

OUT_DIR="$(mktemp -d)"
trap 'rm -rf "$OUT_DIR"' EXIT

"$SCRIPT" --out-dir "$OUT_DIR/skip" >/tmp/remoteapp-multi-window-tracking-skip.out
grep -q '"status": "skipped"' "$OUT_DIR/skip/report.json" || \
  fail "default mode must emit skipped report"
grep -q "skipped" /tmp/remoteapp-multi-window-tracking-skip.out || \
  fail "default output must be explicit skipped evidence"

EASYNET_REMOTEAPP_MULTI_WINDOW_TRACKING_OUT_DIR="$OUT_DIR/good" "$SCRIPT" --self-test >/dev/null
if "$SCRIPT" --run --evidence-json "$OUT_DIR/good/evidence.json" \
    --out-dir "$OUT_DIR/self-test-as-live" >/tmp/remoteapp-multi-window-origin.out 2>&1; then
  fail "verifier accepted contract self-test evidence in run mode"
fi
grep -q "evidence_origin must be live_runner" /tmp/remoteapp-multi-window-origin.out || \
  fail "self-test provenance rejection was not explicit"
python3 - "$OUT_DIR/good/evidence.json" <<'PY'
import json
import sys

path = sys.argv[1]
evidence = json.load(open(path, encoding="utf-8"))
evidence["evidence_origin"] = "live_runner"
json.dump(evidence, open(path, "w", encoding="utf-8"), indent=2)
PY

python3 - "$OUT_DIR/good/report.json" <<'PY'
import json
import sys

report = json.load(open(sys.argv[1], encoding="utf-8"))
scenarios = {scenario["scenario"]: scenario for scenario in report["scenarios"]}
assert set(scenarios) == {
    "independent_window_streams",
    "geometry_churn",
    "application_window_set_churn",
    "target_loss_rebind",
    "multi_display_application",
}
independent = scenarios["independent_window_streams"]
assert independent["stream_count"] == 2
assert independent["distinct_stream_ids"] == independent["stream_count"]
assert independent["distinct_selected_resource_uras"] == independent["stream_count"]
assert independent["frames_interleaved"] is False
assert independent["cross_stream_sentinel_leakage"] is False
geometry = scenarios["geometry_churn"]
assert "TARGET_MOVED" in geometry["events"]
assert "TARGET_RESIZED" in geometry["events"]
assert geometry["geometry_revision_count"] >= 2
app = scenarios["application_window_set_churn"]
assert app["binding_epoch_after"] > app["binding_epoch_before"]
assert app["frames_rendered_after_rebind"] > 0
assert app["display_fallback_used"] is False
loss = scenarios["target_loss_rebind"]
assert "TARGET_LOST" in loss["events"]
assert loss["rebind_deadline_ms"] > loss["lost_at_ms"]
assert scenarios["multi_display_application"]["status"] == "unsupported"
PY

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/interleaved.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
for scenario in evidence["scenarios"]:
    if scenario["scenario"] == "independent_window_streams":
        scenario["frames_interleaved"] = True
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/interleaved.json" --out-dir "$OUT_DIR/interleaved" >/tmp/remoteapp-multi-window-tracking-interleaved.out 2>&1; then
  fail "verifier accepted interleaved frame evidence"
fi
grep -q "frames_interleaved must be false" /tmp/remoteapp-multi-window-tracking-interleaved.out || \
  fail "interleaved stream failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/no-stream-probe.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
for scenario in evidence["scenarios"]:
    if scenario["scenario"] == "independent_window_streams":
        del scenario["streams"][0]["rendered_frame_probe"]
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/no-stream-probe.json" --out-dir "$OUT_DIR/no-stream-probe" >/tmp/remoteapp-multi-window-tracking-no-stream-probe.out 2>&1; then
  fail "verifier accepted independent stream without decoded frame probe"
fi
grep -q "rendered_frame_probe must be present" /tmp/remoteapp-multi-window-tracking-no-stream-probe.out || \
  fail "missing stream frame probe failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/wrong-stream-probe-source.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
for scenario in evidence["scenarios"]:
    if scenario["scenario"] == "independent_window_streams":
        scenario["streams"][0]["rendered_frame_probe"]["frame_source_id"] = "unrelated-frame-source"
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/wrong-stream-probe-source.json" --out-dir "$OUT_DIR/wrong-stream-probe-source" >/tmp/remoteapp-multi-window-tracking-wrong-stream-probe-source.out 2>&1; then
  fail "verifier accepted stream probe bound to a different frame source"
fi
grep -q "rendered_frame_probe frame_source_id must bind stream frame source" /tmp/remoteapp-multi-window-tracking-wrong-stream-probe-source.out || \
  fail "stream frame-source probe failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/stream-probe-leakage.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
for scenario in evidence["scenarios"]:
    if scenario["scenario"] == "independent_window_streams":
        scenario["streams"][0]["rendered_frame_probe"]["foreign_sentinel_rendered"] = True
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/stream-probe-leakage.json" --out-dir "$OUT_DIR/stream-probe-leakage" >/tmp/remoteapp-multi-window-tracking-stream-probe-leakage.out 2>&1; then
  fail "verifier accepted decoded stream probe with foreign sentinel leakage"
fi
grep -q "rendered_frame_probe foreign_sentinel_rendered must be false" /tmp/remoteapp-multi-window-tracking-stream-probe-leakage.out || \
  fail "stream probe foreign sentinel failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/no-resize.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
for scenario in evidence["scenarios"]:
    if scenario["scenario"] == "geometry_churn":
        scenario["events"] = [{"type": "TARGET_MOVED", "target_geometry_revision": 2}]
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/no-resize.json" --out-dir "$OUT_DIR/no-resize" >/tmp/remoteapp-multi-window-tracking-no-resize.out 2>&1; then
  fail "verifier accepted geometry churn without resize"
fi
grep -q "must include TARGET_RESIZED" /tmp/remoteapp-multi-window-tracking-no-resize.out || \
  fail "missing resize failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/display-fallback.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
for scenario in evidence["scenarios"]:
    if scenario["scenario"] == "application_window_set_churn":
        scenario["first_display_capture_started"] = True
        scenario["display_fallback_used"] = True
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/display-fallback.json" --out-dir "$OUT_DIR/display-fallback" >/tmp/remoteapp-multi-window-tracking-display-fallback.out 2>&1; then
  fail "verifier accepted application churn with display fallback"
fi
grep -q "must not start first-display fallback" /tmp/remoteapp-multi-window-tracking-display-fallback.out || \
  fail "display fallback failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/bad-unsupported.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
for scenario in evidence["scenarios"]:
    if scenario["scenario"] == "multi_display_application":
        scenario["capture_session_started"] = True
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/bad-unsupported.json" --out-dir "$OUT_DIR/bad-unsupported" >/tmp/remoteapp-multi-window-tracking-bad-unsupported.out 2>&1; then
  fail "verifier accepted unsupported multi-display app with started capture session"
fi
grep -q "unsupported multi-display app must not start capture session" /tmp/remoteapp-multi-window-tracking-bad-unsupported.out || \
  fail "unsupported capture-session failure was not explicit"

echo "test_remoteapp_multi_window_tracking_e2e: ok"
