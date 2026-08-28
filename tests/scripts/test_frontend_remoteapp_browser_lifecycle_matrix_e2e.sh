#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/tools/scripts/frontend-remoteapp-browser-lifecycle-matrix-e2e.sh"
OUT_DIR="$(mktemp -d)"
trap 'rm -rf "$OUT_DIR"' EXIT

fail() {
  echo "test_frontend_remoteapp_browser_lifecycle_matrix_e2e: $1" >&2
  exit 1
}

"$SCRIPT" --self-test --out-dir "$OUT_DIR/self-test" >/dev/null
grep -q '"status": "passed"' "$OUT_DIR/self-test/report.json" || \
  fail "matrix self-test did not pass"
grep -q '"evidence_origin": "contract_self_test"' "$OUT_DIR/self-test/report.json" || \
  fail "matrix self-test must remain contract-only"

"$SCRIPT" --out-dir "$OUT_DIR/skip" >/dev/null
grep -q '"status": "skipped"' "$OUT_DIR/skip/report.json" || \
  fail "default mode must skip"

python3 - "$OUT_DIR/self-test" <<'PY'
import json
import pathlib
import sys
root = pathlib.Path(sys.argv[1])
for name in ("window", "application"):
    for suffix in ("leaf-report.json", "leaf-evidence.json"):
        path = root / f"{name}-{suffix}"
        value = json.loads(path.read_text(encoding="utf-8"))
        value["evidence_origin"] = "live_runner"
        path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
PY
"$SCRIPT" --run \
  --window-report "$OUT_DIR/self-test/window-leaf-report.json" \
  --application-report "$OUT_DIR/self-test/application-leaf-report.json" \
  --out-dir "$OUT_DIR/live" >/dev/null
grep -q '"interactive_target_kinds": \[' "$OUT_DIR/live/report.json" || \
  fail "live matrix did not expose both target kinds"

python3 - "$OUT_DIR/self-test" <<'PY'
import json
import pathlib
import sys
root = pathlib.Path(sys.argv[1])
for name in ("window", "application"):
    path = root / f"{name}-leaf-report.json"
    value = json.loads(path.read_text(encoding="utf-8"))
    value["input_result"] = "policy_blocked"
    value["input_interaction_sequence_verified"] = False
    value["focus_recovery_verified"] = False
    value["interactive_target_kinds"] = []
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
PY
"$SCRIPT" --run \
  --window-report "$OUT_DIR/self-test/window-leaf-report.json" \
  --application-report "$OUT_DIR/self-test/application-leaf-report.json" \
  --expected-input-mode view_only \
  --out-dir "$OUT_DIR/view-only" >/dev/null
grep -q '"view_only_target_kinds": \[' "$OUT_DIR/view-only/report.json" || \
  fail "view-only matrix did not expose both policy-blocked target kinds"

python3 - "$OUT_DIR/self-test" <<'PY'
import json
import pathlib
import sys
root = pathlib.Path(sys.argv[1])
for name in ("window", "application"):
    path = root / f"{name}-leaf-report.json"
    value = json.loads(path.read_text(encoding="utf-8"))
    value["input_result"] = "input_applied"
    value["input_interaction_sequence_verified"] = True
    value["focus_recovery_verified"] = True
    value["interactive_target_kinds"] = [name]
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
PY

python3 - "$OUT_DIR/self-test/application-leaf-report.json" <<'PY'
import json
import pathlib
import sys
path = pathlib.Path(sys.argv[1])
value = json.loads(path.read_text(encoding="utf-8"))
value["focus_recovery_verified"] = False
path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
PY
if "$SCRIPT" --run \
    --window-report "$OUT_DIR/self-test/window-leaf-report.json" \
    --application-report "$OUT_DIR/self-test/application-leaf-report.json" \
    --out-dir "$OUT_DIR/missing-focus" >/tmp/frontend-remoteapp-browser-matrix-focus.out 2>&1; then
  fail "matrix accepted application target without focus recovery"
fi
grep -q "application report must verify target-blur focus recovery" \
  /tmp/frontend-remoteapp-browser-matrix-focus.out || \
  fail "missing application focus recovery failure was not explicit"

echo "test_frontend_remoteapp_browser_lifecycle_matrix_e2e: ok"
