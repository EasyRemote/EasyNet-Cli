#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/tools/scripts/host-remoteapp-media-adaptation-e2e.sh"
AGGREGATOR="$ROOT/tools/scripts/aggregate-remoteapp-media-adaptation-evidence.py"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

FAKE_RUNNER="$TMP_DIR/fake-browser-runner.sh"
FAKE_AGGREGATOR="$TMP_DIR/fake-aggregator.py"
FIXTURE_LOG="$TMP_DIR/fixture.log"

cat >"$FAKE_RUNNER" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
python3 - "$EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_EVIDENCE_JSON" \
  "$EASYNET_REMOTEAPP_BROWSER_MEDIA_SCENARIO" \
  "${EASYNET_REMOTEAPP_BROWSER_IMPAIRMENT_COMMAND:-}" <<'PY'
import json
import pathlib
import sys

output, scenario, impairment = sys.argv[1:]
if scenario == __import__("os").environ.get("FAKE_RUNNER_FAIL_SCENARIO"):
    raise SystemExit(f"forced {scenario} failure")
if scenario == "baseline" and impairment:
    raise SystemExit("baseline unexpectedly received impairment")
if scenario != "baseline" and not impairment:
    raise SystemExit(f"{scenario} did not receive impairment")
pathlib.Path(output).write_text(json.dumps({
    "status": "passed",
    "scenario": scenario,
    "impairment_present": bool(impairment),
}) + "\n", encoding="utf-8")
PY
SH
chmod +x "$FAKE_RUNNER"

cat >"$FAKE_AGGREGATOR" <<'PY'
#!/usr/bin/env python3
import argparse
import json
import pathlib

parser = argparse.ArgumentParser()
parser.add_argument("--baseline", type=pathlib.Path, required=True)
parser.add_argument("--degraded-network", type=pathlib.Path, required=True)
parser.add_argument("--backpressure", type=pathlib.Path, required=True)
parser.add_argument("--output", type=pathlib.Path, required=True)
args = parser.parse_args()
sources = [
    json.loads(args.baseline.read_text()),
    json.loads(args.degraded_network.read_text()),
    json.loads(args.backpressure.read_text()),
]
assert [item["scenario"] for item in sources] == [
    "baseline", "degraded_network", "backpressure",
]
assert sources[0]["impairment_present"] is False
assert sources[1]["impairment_present"] is True
assert sources[2]["impairment_present"] is True
args.output.write_text(json.dumps({"status": "passed", "scenarios": sources}) + "\n")
PY
chmod +x "$FAKE_AGGREGATOR"

fixture_command() {
  local name="$1"
  local command
  printf -v command "printf '%%s\\\\n' %q >> %q" "$name" "$FIXTURE_LOG"
  printf '%s' "$command"
}

EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_RUNNER_CMD="$FAKE_RUNNER" \
EASYNET_REMOTEAPP_MEDIA_AGGREGATOR="$FAKE_AGGREGATOR" \
EASYNET_REMOTEAPP_MEDIA_BASELINE_PREPARE_COMMAND="$(fixture_command baseline-clean)" \
EASYNET_REMOTEAPP_MEDIA_BASELINE_RESET_COMMAND="$(fixture_command baseline-reset)" \
EASYNET_REMOTEAPP_MEDIA_DEGRADED_NETWORK_APPLY_COMMAND="$(fixture_command degraded-apply)" \
EASYNET_REMOTEAPP_MEDIA_DEGRADED_NETWORK_RESET_COMMAND="$(fixture_command degraded-reset)" \
EASYNET_REMOTEAPP_MEDIA_BACKPRESSURE_APPLY_COMMAND="$(fixture_command pressure-apply)" \
EASYNET_REMOTEAPP_MEDIA_BACKPRESSURE_RESET_COMMAND="$(fixture_command pressure-reset)" \
"$SCRIPT" \
  --frontend-url http://127.0.0.1:3000 \
  --device-id device-test \
  --out-dir "$TMP_DIR/out" \
  --evidence-json "$TMP_DIR/out/matrix.json" >/dev/null

python3 - "$TMP_DIR/out/matrix.json" "$TMP_DIR/out/fixture-plan.json" <<'PY'
import json
import sys

matrix = json.load(open(sys.argv[1], encoding="utf-8"))
assert matrix["status"] == "passed"
assert len(matrix["scenarios"]) == 3
plan = json.load(open(sys.argv[2], encoding="utf-8"))
assert plan["commands_redacted"] is True
serialized = json.dumps(plan)
for secret_text in ("baseline-clean", "degraded-apply", "pressure-apply"):
    assert secret_text not in serialized
PY

python3 - "$FIXTURE_LOG" <<'PY'
import pathlib
import sys

lines = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8").splitlines()
assert lines == [
    "baseline-clean",
    "baseline-reset",
    "degraded-reset",
    "degraded-reset",
    "pressure-reset",
    "pressure-reset",
], lines
PY

FAILURE_FIXTURE_LOG="$TMP_DIR/failure-fixture.log"
fixture_failure_command() {
  local name="$1"
  local command
  printf -v command "printf '%%s\\\\n' %q >> %q" "$name" "$FAILURE_FIXTURE_LOG"
  printf '%s' "$command"
}

if FAKE_RUNNER_FAIL_SCENARIO=backpressure \
  EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_RUNNER_CMD="$FAKE_RUNNER" \
  EASYNET_REMOTEAPP_MEDIA_AGGREGATOR="$FAKE_AGGREGATOR" \
  EASYNET_REMOTEAPP_MEDIA_BASELINE_PREPARE_COMMAND="$(fixture_failure_command baseline-clean)" \
  EASYNET_REMOTEAPP_MEDIA_BASELINE_RESET_COMMAND="$(fixture_failure_command baseline-reset)" \
  EASYNET_REMOTEAPP_MEDIA_DEGRADED_NETWORK_APPLY_COMMAND="$(fixture_failure_command degraded-apply)" \
  EASYNET_REMOTEAPP_MEDIA_DEGRADED_NETWORK_RESET_COMMAND="$(fixture_failure_command degraded-reset)" \
  EASYNET_REMOTEAPP_MEDIA_BACKPRESSURE_APPLY_COMMAND="$(fixture_failure_command pressure-apply)" \
  EASYNET_REMOTEAPP_MEDIA_BACKPRESSURE_RESET_COMMAND="$(fixture_failure_command pressure-reset)" \
  "$SCRIPT" --frontend-url http://127.0.0.1:3000 --device-id device-test \
    --out-dir "$TMP_DIR/forced-failure" >/dev/null 2>&1; then
  echo "runner unexpectedly passed a forced Browser failure" >&2
  exit 1
fi

python3 - "$FAILURE_FIXTURE_LOG" <<'PY'
import pathlib
import sys

lines = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8").splitlines()
assert lines[-2:] == ["pressure-reset", "pressure-reset"], lines
PY

python3 - "$AGGREGATOR" <<'PY'
import importlib.util
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
spec = importlib.util.spec_from_file_location("remoteapp_media_aggregator", path)
module = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(module)

subject = "easynet:///r/test/resource/application.editor"
session = "session-1"
pipeline = "openh264-xcap"

def sample(transport_epoch, media_source_epoch, event_type):
    return {
        "contract": module.PIPELINE_CONTRACT,
        "selected_resource_ura": subject,
        "session_id": session,
        "media_pipeline_id": pipeline,
        "transport_epoch": transport_epoch,
        "media_source_epoch": media_source_epoch,
        "sampled_at_ms": 200,
        "terminal": False,
        "adaptation_events": [{
            "event_type": event_type,
            "sequence": media_source_epoch,
            "observed_at_ms": 180,
            "selected_resource_ura": subject,
            "session_id": session,
            "media_pipeline_id": pipeline,
            "transport_epoch": transport_epoch,
            "media_source_epoch": media_source_epoch,
        }],
    }

old_generation = sample(3, 9, "old_generation_event")
new_generation = sample(3, 10, "new_generation_event")
wrong_transport = sample(2, 10, "old_transport_event")
unique = module.unique_native_samples({"samples": [old_generation, new_generation]})
assert len(unique) == 2, "media_source_epoch must participate in sample identity"
events = module.adaptation_events(
    [old_generation, new_generation, wrong_transport],
    scenario_started_at_ms=100,
    impairment_applied_at_ms=120,
    render_probe_at_ms=220,
    subject_ura=subject,
    session_id=session,
    media_pipeline_id=pipeline,
    transport_epoch=3,
    media_source_epoch=10,
)
assert [event["type"] for event in events] == ["new_generation_event"], events
PY

if EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_RUNNER_CMD="$FAKE_RUNNER" \
  EASYNET_REMOTEAPP_MEDIA_AGGREGATOR="$FAKE_AGGREGATOR" \
  EASYNET_REMOTEAPP_MEDIA_BASELINE_PREPARE_COMMAND=true \
  EASYNET_REMOTEAPP_MEDIA_BASELINE_RESET_COMMAND=true \
  EASYNET_REMOTEAPP_MEDIA_DEGRADED_NETWORK_APPLY_COMMAND=true \
  EASYNET_REMOTEAPP_MEDIA_DEGRADED_NETWORK_RESET_COMMAND=true \
  EASYNET_REMOTEAPP_MEDIA_BACKPRESSURE_APPLY_COMMAND=true \
  "$SCRIPT" --frontend-url http://127.0.0.1:3000 --device-id device-test \
    --out-dir "$TMP_DIR/missing-reset" >/dev/null 2>&1; then
  echo "runner accepted a missing backpressure reset command" >&2
  exit 1
fi

echo "test_host_remoteapp_media_adaptation_e2e: ok"
