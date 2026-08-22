#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/tools/scripts/remoteapp-network-fallback-e2e.sh"

fail() {
  printf 'test_remoteapp_network_fallback_e2e: %s\n' "$1" >&2
  exit 1
}

"$SCRIPT" --self-test >/dev/null

OUT_DIR="$(mktemp -d)"
trap 'rm -rf "$OUT_DIR"' EXIT

"$SCRIPT" --out-dir "$OUT_DIR/skip" >/tmp/remoteapp-network-fallback-skip.out
grep -q '"status": "skipped"' "$OUT_DIR/skip/report.json" || \
  fail "default mode must emit skipped report"
grep -q "skipped" /tmp/remoteapp-network-fallback-skip.out || \
  fail "default output must be explicit skipped evidence"

EASYNET_REMOTEAPP_NETWORK_FALLBACK_OUT_DIR="$OUT_DIR/good" "$SCRIPT" --self-test >/dev/null

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/missing-turn.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["scenarios"] = [
    scenario for scenario in evidence["scenarios"]
    if scenario["route_kind"] != "turn_relay"
]
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/missing-turn.json" --out-dir "$OUT_DIR/missing-turn" >/tmp/remoteapp-network-fallback-missing-turn.out 2>&1; then
  fail "verifier accepted evidence without TURN relay scenario"
fi
grep -q "missing route scenarios: turn_relay" /tmp/remoteapp-network-fallback-missing-turn.out || \
  fail "missing TURN relay failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/leaked-secret.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["scenarios"][2]["turn_password"] = "not-redacted"
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/leaked-secret.json" --out-dir "$OUT_DIR/leaked-secret" >/tmp/remoteapp-network-fallback-secret.out 2>&1; then
  fail "verifier accepted evidence with raw TURN secret"
fi
grep -q "raw credential/secret fields are forbidden" /tmp/remoteapp-network-fallback-secret.out || \
  fail "credential leakage failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/no-media.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["scenarios"][0]["media"]["frames_rendered"] = 0
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/no-media.json" --out-dir "$OUT_DIR/no-media" >/tmp/remoteapp-network-fallback-no-media.out 2>&1; then
  fail "verifier accepted evidence without rendered media frames"
fi
grep -q "media.frames_rendered must be positive" /tmp/remoteapp-network-fallback-no-media.out || \
  fail "rendered media failure was not explicit"

echo "test_remoteapp_network_fallback_e2e: ok"
