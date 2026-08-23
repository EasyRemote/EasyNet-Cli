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
