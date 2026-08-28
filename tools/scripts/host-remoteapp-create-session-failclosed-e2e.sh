#!/usr/bin/env bash
# Host-side remoteapp create_session fail-closed E2E.
#
# Boundary:
# - This script proves SPEC E2E-05 at the public CLI/daemon boundary:
#   live target inventory -> selected window Resource URA -> close selected
#   native window -> create_session failure -> show_session absence probe.
# - It does not validate media or decoded pixels. Media isolation remains owned
#   by host-remoteapp-decoded-frame-e2e.sh.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
BUNDLED_SENTINEL_FIXTURE="$REPO_ROOT/tools/scripts/host-remoteapp-sentinel-fixture.sh"

MODE=run
SCENARIO=stale-window
OUT_DIR=""
SENTINEL_FIXTURE=0
SENTINEL_FIXTURE_CMD="${EASYNET_REMOTEAPP_SENTINEL_FIXTURE_CMD:-}"

usage() {
  cat <<'USAGE'
Usage:
  host-remoteapp-create-session-failclosed-e2e.sh --run --scenario stale-window --sentinel-fixture
  host-remoteapp-create-session-failclosed-e2e.sh --self-test

Options:
  --run                 Execute against the local EasyNet daemon.
  --self-test           Validate the harness against synthetic positive evidence.
  --scenario NAME       Currently only stale-window.
  --sentinel-fixture    Launch the bundled native AppKit selected/unrelated
                        window fixture and use its selected-window close control.
  --sentinel-fixture-cmd CMD
                        Override fixture command. Receives
                        EASYNET_REMOTEAPP_SENTINEL_FIXTURE_DIR and must write
                        env.sh plus cleanup.sh.
  --out-dir DIR         Report directory. Defaults under target/e2e.

Environment:
  EASYNET_REMOTEAPP_EASYNET_BIN
                        Optional easynet binary override.
  EASYNET_REMOTEAPP_SENTINEL_FIXTURE_CMD
                        Same as --sentinel-fixture-cmd.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --run) MODE=run; shift ;;
    --self-test) MODE=self-test; shift ;;
    --scenario)
      case "${2:?missing value for --scenario}" in
        stale-window) SCENARIO="$2" ;;
        *) echo "invalid create-session failclosed scenario: $2" >&2; exit 64 ;;
      esac
      shift 2
      ;;
    --sentinel-fixture) SENTINEL_FIXTURE=1; shift ;;
    --sentinel-fixture-cmd)
      SENTINEL_FIXTURE=1
      SENTINEL_FIXTURE_CMD="${2:?missing value for --sentinel-fixture-cmd}"
      shift 2
      ;;
    --out-dir) OUT_DIR="${2:?missing value for --out-dir}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

TIMESTAMP="$(date -u +%Y%m%d-%H%M%S)"
if [[ -z "$OUT_DIR" ]]; then
  OUT_DIR="$REPO_ROOT/target/e2e/host-remoteapp-create-session-failclosed/$TIMESTAMP-$SCENARIO-$$"
fi
mkdir -p "$OUT_DIR"

EVIDENCE_JSON="$OUT_DIR/evidence.json"
REPORT_JSON="$OUT_DIR/report.json"
REPORT_MD="$OUT_DIR/report.md"
LIVE_INVENTORY_JSON="$OUT_DIR/live-inventory.json"
SELECTED_RESOURCE_JSON="$OUT_DIR/selected-resource.json"
CREATE_STDOUT="$OUT_DIR/create-session.stdout"
CREATE_STDERR="$OUT_DIR/create-session.stderr"
SHOW_STDOUT="$OUT_DIR/show-session.stdout"
SHOW_STDERR="$OUT_DIR/show-session.stderr"
ABILITY_LIST_JSON="$OUT_DIR/ability-list.json"
SESSION_ID="rd-stale-window-e2e-$$"

die() {
  echo "[FAIL] $*" >&2
  exit 1
}

run_easynet() {
  if [[ -n "${EASYNET_REMOTEAPP_EASYNET_BIN:-}" ]]; then
    "$EASYNET_REMOTEAPP_EASYNET_BIN" "$@"
  elif [[ -x "$REPO_ROOT/target/debug/easynet" ]]; then
    "$REPO_ROOT/target/debug/easynet" "$@"
  else
    cargo run --quiet --bin easynet -- "$@"
  fi
}

validate_evidence() {
  python3 - "$EVIDENCE_JSON" "$REPORT_JSON" "$REPORT_MD" <<'PY'
import json
import pathlib
import sys

evidence_path, report_path, md_path = sys.argv[1:4]
with open(evidence_path, encoding="utf-8") as f:
    evidence = json.load(f)

errors = []

def require(condition, message):
    if not condition:
        errors.append(message)

def get(path, default=None):
    value = evidence
    for part in path.split("."):
        if not isinstance(value, dict) or part not in value:
            return default
        value = value[part]
    return value

def contains_subject_arg(value):
    if isinstance(value, dict):
        for key, child in value.items():
            if key in {"subject", "subject_ura", "resource_ura"}:
                return True
            if contains_subject_arg(child):
                return True
    elif isinstance(value, list):
        return any(contains_subject_arg(child) for child in value)
    return False

selected = get("selected_resource_ura")
create = get("create_session", {})
show = get("show_session_absence_probe", {})
args = create.get("args") if isinstance(create, dict) else None
stderr = create.get("stderr", "") if isinstance(create, dict) else ""
show_stderr = show.get("stderr", "") if isinstance(show, dict) else ""

require(evidence.get("status") == "passed", "evidence.status must be passed")
require(evidence.get("scenario") == "stale-window", "scenario must be stale-window")
require(isinstance(selected, str) and selected.startswith("easynet:///"),
        "selected_resource_ura must be an EasyNet Resource URA")
require(get("selected_target.type") == "window", "selected target must be a window")
require(get("selected_target.metadata.availability") == "available",
        "selected target must come from live available inventory before close")
require(get("selected_target.metadata.freshness.source") == "live_refresh",
        "selected target must come from live_refresh inventory")
require(get("host_action.action") == "close",
        "host action must close the selected native window before create_session")
require(get("host_action.ack") == "close",
        "selected native window close must be acknowledged by the fixture")
require(create.get("ability") == "remote_desktop.create_session",
        "create_session ability must be remote_desktop.create_session")
require(create.get("subject_ura") == selected,
        "create_session Invocation.subject must equal selected_resource_ura")
require(isinstance(args, dict), "create_session args must be recorded as an object")
if isinstance(args, dict):
    require(not contains_subject_arg(args),
            "create_session args must not contain subject, subject_ura, or resource_ura")
    require(isinstance(args.get("session_id"), str) and args.get("session_id"),
            "create_session args must include deterministic session_id for absence probe")
require(create.get("exit_code") not in (None, 0),
        "stale-window create_session must fail")
require(
    "target_not_found" in stderr or "target_stale" in stderr,
    "stale-window create_session failure must expose target_not_found or target_stale",
)
require("refresh_targets" in stderr,
        "stale-window create_session failure must expose frontend_action=refresh_targets")
require(evidence.get("active_session_row_inserted") is False,
        "stale-window failure must prove no active session row was inserted")
require(show.get("ability") == "remote_desktop.show_session",
        "absence probe must use remote_desktop.show_session")
require(show.get("subject_ura") == selected,
        "show_session absence probe must use the selected Resource URA as subject")
require(show.get("exit_code") not in (None, 0),
        "show_session absence probe must fail for a non-inserted session")
require("session_not_found" in show_stderr,
        "show_session absence probe must fail with session_not_found")
require("session_token_mismatch" not in show_stderr,
        "absence probe must not reach token validation for an inserted session row")

report = {
    "status": "failed" if errors else "passed",
    "errors": errors,
    "evidence_json": evidence_path,
    "scenario": evidence.get("scenario"),
    "selected_resource_ura": selected,
    "create_session_exit_code": create.get("exit_code") if isinstance(create, dict) else None,
    "show_session_exit_code": show.get("exit_code") if isinstance(show, dict) else None,
}
pathlib.Path(report_path).write_text(
    json.dumps(report, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
with open(md_path, "w", encoding="utf-8") as f:
    f.write("# RemoteApp create_session fail-closed E2E report\n\n")
    f.write(f"- Status: `{report['status']}`\n")
    f.write(f"- Scenario: `{report['scenario']}`\n")
    f.write(f"- Evidence: `{evidence_path}`\n")
    if errors:
        f.write("\n## Errors\n")
        for error in errors:
            f.write(f"- {error}\n")
if errors:
    for error in errors:
        print(error, file=sys.stderr)
    raise SystemExit(1)
PY
}

if [[ "$MODE" == "self-test" ]]; then
  python3 - "$EVIDENCE_JSON" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
subject = "easynet:///r/localhost/resource/device.dev/streams/window.stale"
evidence = {
    "status": "passed",
    "scenario": "stale-window",
    "selected_resource_ura": subject,
    "selected_target": {
        "type": "window",
        "metadata": {
            "availability": "available",
            "freshness": {"source": "live_refresh"},
        },
    },
    "host_action": {"action": "close", "ack": "close"},
    "create_session": {
        "ability": "remote_desktop.create_session",
        "subject_ura": subject,
        "args": {
            "session_id": "rd-stale-window-e2e",
            "mode": "view_only",
            "transport_preferences": ["webrtc"],
        },
        "exit_code": 1,
        "stderr": "target_not_found; frontend_action=refresh_targets",
    },
    "show_session_absence_probe": {
        "ability": "remote_desktop.show_session",
        "subject_ura": subject,
        "exit_code": 1,
        "stderr": 'session "rd-stale-window-e2e" not found; reason=session_not_found',
    },
    "active_session_row_inserted": False,
}
path.write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
  validate_evidence
  echo "host-remoteapp-create-session-failclosed-e2e self-test ok"
  exit 0
fi

[[ "$SCENARIO" == "stale-window" ]] || die "unsupported scenario: $SCENARIO"
[[ "$SENTINEL_FIXTURE" == "1" ]] || die "--sentinel-fixture is required for live stale-window E2E"
if [[ -z "$SENTINEL_FIXTURE_CMD" ]]; then
  [[ -x "$BUNDLED_SENTINEL_FIXTURE" ]] || die "missing bundled sentinel fixture: $BUNDLED_SENTINEL_FIXTURE"
  SENTINEL_FIXTURE_CMD="$BUNDLED_SENTINEL_FIXTURE --target-kind window"
fi

SENTINEL_FIXTURE_DIR="$OUT_DIR/sentinel-fixture"
mkdir -p "$SENTINEL_FIXTURE_DIR"
export EASYNET_REMOTEAPP_SENTINEL_FIXTURE_DIR="$SENTINEL_FIXTURE_DIR"
trap '[[ -x "$SENTINEL_FIXTURE_DIR/cleanup.sh" ]] && "$SENTINEL_FIXTURE_DIR/cleanup.sh" >/dev/null 2>&1 || true' EXIT
bash -lc "$SENTINEL_FIXTURE_CMD"
[[ -f "$SENTINEL_FIXTURE_DIR/env.sh" ]] || die "sentinel fixture did not write env.sh"
source "$SENTINEL_FIXTURE_DIR/env.sh"
[[ -x "${EASYNET_REMOTEAPP_SELECTED_CONTROL_SH:-}" ]] || die "sentinel fixture did not export selected control helper"
[[ -n "${EASYNET_REMOTEAPP_TARGET_PID:-}" ]] || die "sentinel fixture did not export selected target pid"

run_easynet ability refresh-remote-targets --type window --format json >"$LIVE_INVENTORY_JSON"
python3 "$SELF_DIR/remoteapp-select-live-target.py" \
  --inventory "$LIVE_INVENTORY_JSON" \
  --output "$SELECTED_RESOURCE_JSON" \
  --kind window \
  --pid "$EASYNET_REMOTEAPP_TARGET_PID"

SELECTED_RESOURCE_URA="$(python3 - "$SELECTED_RESOURCE_JSON" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as f:
    print(json.load(f)["resource_ura"])
PY
)"

"$EASYNET_REMOTEAPP_SELECTED_CONTROL_SH" close
HOST_ACTION_ACK="$(tr -d '[:space:]' < "$SENTINEL_FIXTURE_DIR/selected-ack.txt")"
sleep 0.5

set +e
run_easynet ability create-remote-desktop-session \
  --subject "$SELECTED_RESOURCE_URA" \
  --session-id "$SESSION_ID" \
  --mode view_only \
  --transport webrtc \
  --format json >"$CREATE_STDOUT" 2>"$CREATE_STDERR"
CREATE_EXIT_CODE=$?
set -e

run_easynet ability list --format json >"$ABILITY_LIST_JSON"
SHOW_SESSION_ABILITY_URA="$(python3 - "$ABILITY_LIST_JSON" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as f:
    for ability in json.load(f):
        if ability.get("name") == "remote_desktop.show_session":
            print(ability["ability_ura"])
            raise SystemExit(0)
raise SystemExit("remote_desktop.show_session ability not found")
PY
)"
NONCE_HEX="$(python3 - "$SESSION_ID" <<'PY'
import hashlib
import sys
print(hashlib.sha256(sys.argv[1].encode("utf-8")).hexdigest()[:32])
PY
)"

set +e
run_easynet ability invoke "$SHOW_SESSION_ABILITY_URA" \
  --subject "$SELECTED_RESOURCE_URA" \
  --args "{\"session_id\":\"$SESSION_ID\",\"session_token\":\"invalid\"}" \
  --causal-root \
  --nonce-hex "$NONCE_HEX" \
  --timeout 5 \
  --raw >"$SHOW_STDOUT" 2>"$SHOW_STDERR"
SHOW_EXIT_CODE=$?
set -e

python3 - "$EVIDENCE_JSON" "$SELECTED_RESOURCE_JSON" "$SELECTED_RESOURCE_URA" "$SESSION_ID" \
  "$CREATE_EXIT_CODE" "$CREATE_STDOUT" "$CREATE_STDERR" "$SHOW_EXIT_CODE" "$SHOW_STDOUT" "$SHOW_STDERR" \
  "$HOST_ACTION_ACK" <<'PY'
import json
import pathlib
import sys

(
    evidence_path,
    selected_path,
    selected_ura,
    session_id,
    create_exit_code,
    create_stdout_path,
    create_stderr_path,
    show_exit_code,
    show_stdout_path,
    show_stderr_path,
    host_action_ack,
) = sys.argv[1:12]

with open(selected_path, encoding="utf-8") as f:
    selected = json.load(f)
create_stderr = pathlib.Path(create_stderr_path).read_text(errors="replace")
show_stderr = pathlib.Path(show_stderr_path).read_text(errors="replace")
session_not_found = "session_not_found" in show_stderr
evidence = {
    "status": "passed",
    "scenario": "stale-window",
    "selected_resource_ura": selected_ura,
    "selected_target": selected,
    "host_action": {
        "action": "close",
        "ack": host_action_ack,
    },
    "create_session": {
        "ability": "remote_desktop.create_session",
        "subject_ura": selected_ura,
        "args": {
            "session_id": session_id,
            "mode": "view_only",
            "transport_preferences": ["webrtc"],
        },
        "exit_code": int(create_exit_code),
        "stdout": pathlib.Path(create_stdout_path).read_text(errors="replace"),
        "stderr": create_stderr,
    },
    "show_session_absence_probe": {
        "ability": "remote_desktop.show_session",
        "subject_ura": selected_ura,
        "exit_code": int(show_exit_code),
        "stdout": pathlib.Path(show_stdout_path).read_text(errors="replace"),
        "stderr": show_stderr,
    },
    "active_session_row_inserted": not session_not_found,
}
pathlib.Path(evidence_path).write_text(
    json.dumps(evidence, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
PY

validate_evidence
echo "PASS: $REPORT_MD"
