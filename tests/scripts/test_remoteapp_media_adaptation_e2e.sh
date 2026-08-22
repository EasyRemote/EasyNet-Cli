#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/tools/scripts/remoteapp-media-adaptation-e2e.sh"

fail() {
  printf 'test_remoteapp_media_adaptation_e2e: %s\n' "$1" >&2
  exit 1
}

"$SCRIPT" --self-test >/dev/null

OUT_DIR="$(mktemp -d)"
trap 'rm -rf "$OUT_DIR"' EXIT

"$SCRIPT" --out-dir "$OUT_DIR/skip" >/tmp/remoteapp-media-adaptation-skip.out
grep -q '"status": "skipped"' "$OUT_DIR/skip/report.json" || \
  fail "default mode must emit skipped report"
grep -q "skipped" /tmp/remoteapp-media-adaptation-skip.out || \
  fail "default output must be explicit skipped evidence"

EASYNET_REMOTEAPP_MEDIA_ADAPTATION_OUT_DIR="$OUT_DIR/good" "$SCRIPT" --self-test >/dev/null

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/no-codec.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["scenarios"][0]["video"]["codec_negotiated"] = False
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/no-codec.json" --out-dir "$OUT_DIR/no-codec" >/tmp/remoteapp-media-adaptation-no-codec.out 2>&1; then
  fail "verifier accepted media evidence without negotiated video codec"
fi
grep -q "video.codec_negotiated must be true" /tmp/remoteapp-media-adaptation-no-codec.out || \
  fail "missing video codec failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/no-audio.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["scenarios"][0]["audio"]["packets_rendered"] = 0
evidence["scenarios"][0]["audio"]["samples_rendered"] = 0
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/no-audio.json" --out-dir "$OUT_DIR/no-audio" >/tmp/remoteapp-media-adaptation-no-audio.out 2>&1; then
  fail "verifier accepted media evidence without rendered audio"
fi
grep -q "audio must render packets or samples" /tmp/remoteapp-media-adaptation-no-audio.out || \
  fail "missing audio render failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/no-adaptation.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
for scenario in evidence["scenarios"]:
    if scenario["scenario"] == "degraded_network":
        scenario["adaptation"]["events"] = [{"type": "steady_state"}]
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/no-adaptation.json" --out-dir "$OUT_DIR/no-adaptation" >/tmp/remoteapp-media-adaptation-no-adaptation.out 2>&1; then
  fail "verifier accepted degraded-network evidence without bitrate downshift"
fi
grep -q "degraded_network must include bitrate_downshift" /tmp/remoteapp-media-adaptation-no-adaptation.out || \
  fail "missing adaptation failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/unbounded-queue.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["scenarios"][0]["queue"]["observed_max_depth"] = 9
evidence["scenarios"][0]["drop_policy"]["unbounded_queue"] = True
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/unbounded-queue.json" --out-dir "$OUT_DIR/unbounded-queue" >/tmp/remoteapp-media-adaptation-unbounded-queue.out 2>&1; then
  fail "verifier accepted unbounded media queue evidence"
fi
grep -q "queue.observed_max_depth must not exceed max_depth" /tmp/remoteapp-media-adaptation-unbounded-queue.out || \
  fail "unbounded queue failure was not explicit"

echo "test_remoteapp_media_adaptation_e2e: ok"
