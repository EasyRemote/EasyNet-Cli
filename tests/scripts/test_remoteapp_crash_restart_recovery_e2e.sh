#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh"

fail() {
  printf 'test_remoteapp_crash_restart_recovery_e2e: %s\n' "$1" >&2
  exit 1
}

"$SCRIPT" --self-test >/dev/null

OUT_DIR="$(mktemp -d)"
trap 'rm -rf "$OUT_DIR"' EXIT

"$SCRIPT" --out-dir "$OUT_DIR/skip" >/tmp/remoteapp-crash-restart-recovery-skip.out
grep -q '"status": "skipped"' "$OUT_DIR/skip/report.json" || \
  fail "default mode must emit skipped report"
grep -q "skipped" /tmp/remoteapp-crash-restart-recovery-skip.out || \
  fail "default output must be explicit skipped evidence"

EASYNET_REMOTEAPP_CRASH_RESTART_RECOVERY_OUT_DIR="$OUT_DIR/good" "$SCRIPT" --self-test >/dev/null

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/no-replay-guard.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["scenarios"][0]["recovery"]["replay_guard_recovered"] = False
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/no-replay-guard.json" --out-dir "$OUT_DIR/no-replay-guard" >/tmp/remoteapp-crash-restart-no-replay-guard.out 2>&1; then
  fail "verifier accepted crash recovery without replay guard recovery"
fi
grep -q "replay_guard_recovered must be true" /tmp/remoteapp-crash-restart-no-replay-guard.out || \
  fail "replay guard failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/new-session.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["scenarios"][0]["after_restart"]["session_id"] = "sess-replaced"
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/new-session.json" --out-dir "$OUT_DIR/new-session" >/tmp/remoteapp-crash-restart-new-session.out 2>&1; then
  fail "verifier accepted daemon restart that replaced the public session"
fi
grep -q "session_id must remain stable across restart" /tmp/remoteapp-crash-restart-new-session.out || \
  fail "same-session failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/no-media.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["scenarios"][0]["after_restart"]["frames_rendered_after_restart"] = 0
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/no-media.json" --out-dir "$OUT_DIR/no-media" >/tmp/remoteapp-crash-restart-no-media.out 2>&1; then
  fail "verifier accepted restart recovery without rendered media"
fi
grep -q "frames_rendered_after_restart must be positive" /tmp/remoteapp-crash-restart-no-media.out || \
  fail "media reattach failure was not explicit"

python3 - "$OUT_DIR/good/evidence.json" "$OUT_DIR/new-receipt.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
for scenario in evidence["scenarios"]:
    if scenario["scenario"] == "terminal_receipt_replay_after_crash":
        scenario["terminal_receipt_after_restart"]["receipt_id"] = "replacement-receipt"
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
if "$SCRIPT" --run --evidence-json "$OUT_DIR/new-receipt.json" --out-dir "$OUT_DIR/new-receipt" >/tmp/remoteapp-crash-restart-new-receipt.out 2>&1; then
  fail "verifier accepted terminal receipt replacement after crash"
fi
grep -q "terminal receipt id must be replayed" /tmp/remoteapp-crash-restart-new-receipt.out || \
  fail "terminal receipt replay failure was not explicit"

echo "test_remoteapp_crash_restart_recovery_e2e: ok"
