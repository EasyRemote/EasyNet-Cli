#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/tools/scripts/remoteapp-cross-platform-capture-e2e.sh"

fail() {
  printf 'test_remoteapp_cross_platform_capture_e2e: %s\n' "$1" >&2
  exit 1
}

"$SCRIPT" --self-test >/dev/null

OUT_DIR="$(mktemp -d)"
trap 'rm -rf "$OUT_DIR"' EXIT

"$SCRIPT" --out-dir "$OUT_DIR/skip" >/tmp/remoteapp-cross-platform-capture-skip.out
grep -q '"status": "skipped"' "$OUT_DIR/skip/report.json" || \
  fail "default mode must emit skipped report"
grep -q "skipped" /tmp/remoteapp-cross-platform-capture-skip.out || \
  fail "default output must be explicit skipped evidence"

EASYNET_REMOTEAPP_CROSS_PLATFORM_CAPTURE_OUT_DIR="$OUT_DIR/good" "$SCRIPT" --self-test >/dev/null
python3 - "$OUT_DIR/good/report.json" <<'PY'
import json
import sys

report = json.load(open(sys.argv[1], encoding="utf-8"))
platforms = {platform["platform"]: platform for platform in report["platforms"]}
macos_scenarios = {
    scenario["target_kind"]: scenario
    for scenario in platforms["macos"]["scenarios"]
}
for target in ("display", "window", "application"):
    scenario = macos_scenarios[target]
    assert scenario["status"] == "passed"
    assert scenario["selected_resource_ura"].startswith("easynet:///")
    assert scenario["session_id"]
    assert scenario["capture_scope"]
    assert scenario["target_binding_exact"] is True
    assert scenario["source_only_proof"] is False
    assert scenario["frame_source_id"]
    assert scenario["geometry_revision"] > 0
    assert scenario["frames_rendered"] > 0
    assert scenario["selected_sentinel_rendered"] is True
    assert scenario["rendered_frame_probe_bound"] is True
    assert scenario["selected_sentinel_hash_present"] is True
    assert scenario["terminal_receipt_visible"] is True
    assert scenario["terminal_receipt_session_bound"] is True
for target in ("window", "application"):
    assert macos_scenarios[target]["first_display_capture_started"] is False
    assert macos_scenarios[target]["display_fallback_used"] is False
    assert macos_scenarios[target]["unrelated_sentinel_rendered"] is False
PY

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/missing-linux.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["platforms"] = [
    platform for platform in evidence["platforms"]
    if platform["platform"] != "linux"
]
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/missing-linux.json" --out-dir "$OUT_DIR/missing-linux" >/tmp/remoteapp-cross-platform-capture-missing-linux.out 2>&1; then
  fail "verifier accepted evidence without Linux platform entry"
fi
grep -q "missing platform evidence: linux" /tmp/remoteapp-cross-platform-capture-missing-linux.out || \
  fail "missing Linux failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/macos-unsupported.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["platforms"][0]["scenarios"][1] = {
    "target_kind": "window",
    "status": "unsupported",
    "unsupported_state": "explicit_product_unsupported",
    "show_unsupported": True,
    "session_id": None,
    "frames_rendered": 0,
    "first_display_capture_started": False,
}
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/macos-unsupported.json" --out-dir "$OUT_DIR/macos-unsupported" >/tmp/remoteapp-cross-platform-capture-macos-unsupported.out 2>&1; then
  fail "verifier accepted unsupported macOS window capture"
fi
grep -q "macos must pass display/window/application capture" /tmp/remoteapp-cross-platform-capture-macos-unsupported.out || \
  fail "macOS unsupported failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/display-fallback.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["platforms"][0]["scenarios"][1]["first_display_capture_started"] = True
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/display-fallback.json" --out-dir "$OUT_DIR/display-fallback" >/tmp/remoteapp-cross-platform-capture-display-fallback.out 2>&1; then
  fail "verifier accepted window capture that started display fallback"
fi
grep -q "window/application capture must not start first-display fallback" /tmp/remoteapp-cross-platform-capture-display-fallback.out || \
  fail "display fallback failure was not explicit"

echo "test_remoteapp_cross_platform_capture_e2e: ok"
