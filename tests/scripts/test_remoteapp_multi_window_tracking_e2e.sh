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
