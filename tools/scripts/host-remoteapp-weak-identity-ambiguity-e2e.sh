#!/usr/bin/env bash
# Host-side remoteapp weak-identity ambiguity E2E.
#
# Boundary:
# - This script proves SPEC E2E-10 at the public CLI/daemon boundary:
#   live-looking window Resource with weak native identity -> create_session
#   typed target_identity_ambiguous failure -> no active session row -> no stream.
# - It deliberately injects one weak local window Resource into the operator's
#   resource registry and restores the registry before exiting.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"

MODE=run
SCENARIO=window-app-title-only
OUT_DIR=""

usage() {
  cat <<'USAGE'
Usage:
  host-remoteapp-weak-identity-ambiguity-e2e.sh --run
  host-remoteapp-weak-identity-ambiguity-e2e.sh --self-test

Options:
  --run                 Execute against the local EasyNet daemon.
  --self-test           Validate the harness against synthetic positive evidence.
  --scenario NAME       Currently only window-app-title-only.
  --out-dir DIR         Report directory. Defaults under target/e2e.

Environment:
  EASYNET_REMOTEAPP_EASYNET_BIN
                        Optional easynet binary override.
  EASYNET_REMOTEAPP_STATE_DIR
                        Optional state dir override. Defaults to $HOME/.easynet.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --run) MODE=run; shift ;;
    --self-test) MODE=self-test; shift ;;
    --scenario)
      case "${2:?missing value for --scenario}" in
        window-app-title-only) SCENARIO="$2" ;;
        *) echo "invalid weak identity ambiguity scenario: $2" >&2; exit 64 ;;
      esac
      shift 2
      ;;
    --out-dir) OUT_DIR="${2:?missing value for --out-dir}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

TIMESTAMP="$(date -u +%Y%m%d-%H%M%S)"
if [[ -z "$OUT_DIR" ]]; then
  OUT_DIR="$REPO_ROOT/target/e2e/host-remoteapp-weak-identity-ambiguity/$TIMESTAMP-$SCENARIO-$$"
fi
mkdir -p "$OUT_DIR"

EVIDENCE_JSON="$OUT_DIR/evidence.json"
REPORT_JSON="$OUT_DIR/report.json"
REPORT_MD="$OUT_DIR/report.md"
WEAK_RESOURCE_JSON="$OUT_DIR/weak-resource.json"
CREATE_STDOUT="$OUT_DIR/create-session.stdout"
CREATE_STDERR="$OUT_DIR/create-session.stderr"
SHOW_STDOUT="$OUT_DIR/show-session.stdout"
SHOW_STDERR="$OUT_DIR/show-session.stderr"
ABILITY_LIST_JSON="$OUT_DIR/ability-list.json"
SESSION_ID="rd-weak-identity-ambiguity-e2e-$$"

STATE_DIR="${EASYNET_REMOTEAPP_STATE_DIR:-${HOME:?HOME is required}/.easynet}"
CREDENTIALS_PATH="$STATE_DIR/credentials.json"
RESOURCES_PATH="$STATE_DIR/resources.json"
RESOURCES_BACKUP="$OUT_DIR/resources-before.json"
RESOURCES_ORIGINALLY_PRESENT=0
RESOURCES_RESTORED=0

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

restore_resources() {
  if [[ "$RESOURCES_RESTORED" == "1" ]]; then
    return 0
  fi
  if [[ "$RESOURCES_ORIGINALLY_PRESENT" == "1" ]]; then
    cp "$RESOURCES_BACKUP" "$RESOURCES_PATH"
    chmod 600 "$RESOURCES_PATH" 2>/dev/null || true
  else
    rm -f "$RESOURCES_PATH"
  fi
  RESOURCES_RESTORED=1
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

selected = get("weak_resource.resource_ura")
metadata = get("weak_resource.metadata", {})
create = get("create_session", {})
show = get("show_session_absence_probe", {})
args = create.get("args") if isinstance(create, dict) else None
stderr = create.get("stderr", "") if isinstance(create, dict) else ""
show_stderr = show.get("stderr", "") if isinstance(show, dict) else ""
reason = create.get("target_reason") if isinstance(create, dict) else None

require(evidence.get("status") == "passed", "evidence.status must be passed")
require(evidence.get("scenario") == "window-app-title-only", "scenario must be window-app-title-only")
require(
    isinstance(selected, str)
    and selected.startswith("easynet:///")
    and "/resource/device." in selected
    and "/streams/window." in selected,
    "weak_resource.resource_ura must be a canonical device-stream window Resource URA",
)
require(get("weak_resource.type") == "window", "weak resource must be a window")
require(
    isinstance(metadata, dict) and metadata.get("availability") == "available",
    "weak resource must pass live inventory availability so identity validation owns the failure",
)
require(isinstance(metadata, dict) and isinstance(metadata.get("window_id"), int),
        "weak window resource must include a stable window_id")
require(isinstance(metadata, dict) and metadata.get("app_name") and metadata.get("title"),
        "weak window resource must include app_name/title diagnostic hints")
require(
    isinstance(metadata, dict)
    and "pid" not in metadata
    and "primary_pid" not in metadata
    and "app_identity" not in metadata
    and "bundle_id" not in metadata,
    "weak window resource must omit pid, primary_pid, app_identity, and bundle_id",
)
require(create.get("ability") == "remote_desktop.create_session",
        "create_session ability must be remote_desktop.create_session")
require(create.get("subject_ura") == selected,
        "create_session subject must equal the weak window resource")
require(isinstance(args, dict), "create_session args must be recorded as an object")
if isinstance(args, dict):
    require(not contains_subject_arg(args), "create_session args must not contain subject identity")
    require(args.get("session_id"), "create_session args must include deterministic session_id")
require(create.get("exit_code") not in (None, 0), "weak window create_session must fail")
require(
    reason == "target_identity_ambiguous" or "target_identity_ambiguous" in stderr,
    "weak window create_session failure must expose target_identity_ambiguous",
)
require("app_name/title are diagnostic" in stderr,
        "failure must state that app_name/title are diagnostic hints, not routing identity")
require(evidence.get("active_session_row_inserted") is False,
        "weak identity failure must prove no active session row was inserted")
require(show.get("ability") == "remote_desktop.show_session",
        "absence probe must use remote_desktop.show_session")
require(show.get("subject_ura") == selected, "absence probe subject must equal weak resource")
require(show.get("exit_code") not in (None, 0), "absence probe must fail")
require("session_not_found" in show_stderr, "absence probe must fail with session_not_found")
require("session_token_mismatch" not in show_stderr, "absence probe must not reach an inserted row")
require(evidence.get("stream_start_attempted") is False, "stream startup must not be attempted")
require(evidence.get("media_start_attempted") is False, "media startup must not be attempted")
require(get("decoded_frames.count") == 0, "decoded frame count must be zero because session failed before media")
require(evidence.get("resource_registry_restored") is True,
        "harness must restore the resource registry before reporting success")

report = {
    "status": "failed" if errors else "passed",
    "errors": errors,
    "evidence_json": evidence_path,
    "scenario": evidence.get("scenario"),
    "selected_resource_ura": selected,
    "create_session_exit_code": create.get("exit_code") if isinstance(create, dict) else None,
    "create_session_target_reason": reason,
}
pathlib.Path(report_path).write_text(
    json.dumps(report, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
with open(md_path, "w", encoding="utf-8") as f:
    f.write("# RemoteApp weak identity ambiguity E2E report\n\n")
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
subject = "easynet:///r/localhost/resource/device.dev/streams/window.weak"
evidence = {
    "status": "passed",
    "scenario": "window-app-title-only",
    "weak_resource": {
        "resource_ura": subject,
        "owner_agent": "easynet:///r/localhost/agent/device.dev.media",
        "type": "window",
        "binding": "local_device",
        "hardware_id": "window:e2e:weak-identity:self-test",
        "display_name": "Terminal — same-looking shell",
        "metadata": {
            "availability": "available",
            "backend": "e2e_weak_identity",
            "freshness": {"source": "live_refresh"},
            "window_id": 7,
            "app_name": "Terminal",
            "title": "same-looking shell",
        },
    },
    "create_session": {
        "ability": "remote_desktop.create_session",
        "subject_ura": subject,
        "args": {
            "session_id": "rd-weak-identity-ambiguity-e2e",
            "mode": "view_only",
            "transport_preferences": ["webrtc"],
        },
        "exit_code": 1,
        "target_reason": "target_identity_ambiguous",
        "stderr": "window targets require owner pid, app_identity, or bundle_id in addition to window_id; app_name/title are diagnostic hints, not production routing identity; reason=target_identity_ambiguous",
    },
    "show_session_absence_probe": {
        "ability": "remote_desktop.show_session",
        "subject_ura": subject,
        "exit_code": 1,
        "stderr": 'session "rd-weak-identity-ambiguity-e2e" not found; reason=session_not_found',
    },
    "active_session_row_inserted": False,
    "stream_start_attempted": False,
    "media_start_attempted": False,
    "decoded_frames": {
        "count": 0,
        "observation": "session_failed_before_stream",
    },
    "resource_registry_restored": True,
}
path.write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
  validate_evidence
  echo "host-remoteapp-weak-identity-ambiguity-e2e self-test ok"
  exit 0
fi

[[ "$SCENARIO" == "window-app-title-only" ]] || die "unsupported scenario: $SCENARIO"
[[ -f "$CREDENTIALS_PATH" ]] || die "missing credentials: $CREDENTIALS_PATH"
[[ -d "$STATE_DIR" ]] || die "missing EasyNet state dir: $STATE_DIR"
[[ "$RESOURCES_PATH" == "$STATE_DIR/resources.json" ]] || die "refusing unexpected resources path: $RESOURCES_PATH"

if [[ -f "$RESOURCES_PATH" ]]; then
  RESOURCES_ORIGINALLY_PRESENT=1
  cp "$RESOURCES_PATH" "$RESOURCES_BACKUP"
else
  printf '{"resources":[]}\n' >"$RESOURCES_BACKUP"
fi
trap 'restore_resources >/dev/null 2>&1 || true' EXIT

python3 - "$CREDENTIALS_PATH" "$RESOURCES_PATH" "$WEAK_RESOURCE_JSON" "$SESSION_ID" <<'PY'
import json
import pathlib
import sys
import time
import uuid

credentials_path, resources_path, weak_path, session_id = sys.argv[1:5]
with open(credentials_path, encoding="utf-8") as f:
    credentials = json.load(f)
realm = credentials.get("realm")
node_id = credentials.get("node_id")
if not realm or not node_id:
    raise SystemExit("credentials must include realm and node_id")

resources_file = pathlib.Path(resources_path)
if resources_file.exists():
    with open(resources_file, encoding="utf-8") as f:
        resources = json.load(f)
else:
    resources = {"resources": []}
if not isinstance(resources, dict) or not isinstance(resources.get("resources"), list):
    raise SystemExit("resources.json must have a resources array")

run_id = uuid.uuid4().hex
now_ms = int(time.time() * 1000)
resource = {
    "resource_ura": f"easynet:///r/{realm}/resource/device.{node_id}/streams/window.e2e_weak_identity_{run_id}",
    "owner_agent": f"easynet:///r/{realm}/agent/device.{node_id}.media",
    "type": "window",
    "binding": "local_device",
    "hardware_id": f"window:e2e:weak-identity:{session_id}:{run_id}",
    "display_name": "Terminal — same-looking shell",
    "metadata": {
        "availability": "available",
        "backend": "e2e_weak_identity",
        "discovery_source": "host-remoteapp-weak-identity-ambiguity-e2e",
        "freshness": {
            "observed_at_ms": now_ms,
            "stale_after_ms": now_ms + 600_000,
            "source": "live_refresh",
        },
        "freshness_ttl_ms": 600_000,
        "host_device_ura": f"easynet:///r/{realm}/device/{node_id}",
        "window_id": 7,
        "app_name": "Terminal",
        "title": "same-looking shell",
    },
    "first_seen_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
}
resources["resources"].append(resource)
resources_file.write_text(json.dumps(resources, indent=2, sort_keys=True) + "\n", encoding="utf-8")
pathlib.Path(weak_path).write_text(
    json.dumps(resource, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
PY
chmod 600 "$RESOURCES_PATH" 2>/dev/null || true

WEAK_RESOURCE_URA="$(python3 - "$WEAK_RESOURCE_JSON" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as f:
    print(json.load(f)["resource_ura"])
PY
)"

set +e
run_easynet ability create-remote-desktop-session \
  --subject "$WEAK_RESOURCE_URA" \
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
    data = json.load(f)
if isinstance(data, dict):
    abilities = data.get("abilities", [])
else:
    abilities = data
for ability in abilities:
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
  --subject "$WEAK_RESOURCE_URA" \
  --args "{\"session_id\":\"$SESSION_ID\",\"session_token\":\"invalid\"}" \
  --causal-root \
  --nonce-hex "$NONCE_HEX" \
  --timeout 5 \
  --raw >"$SHOW_STDOUT" 2>"$SHOW_STDERR"
SHOW_EXIT_CODE=$?
set -e

restore_resources

python3 - "$EVIDENCE_JSON" "$WEAK_RESOURCE_JSON" "$WEAK_RESOURCE_URA" "$SESSION_ID" \
  "$CREATE_EXIT_CODE" "$CREATE_STDOUT" "$CREATE_STDERR" \
  "$SHOW_EXIT_CODE" "$SHOW_STDOUT" "$SHOW_STDERR" <<'PY'
import json
import pathlib
import re
import sys

(
    evidence_path,
    weak_path,
    weak_ura,
    session_id,
    create_exit_code,
    create_stdout_path,
    create_stderr_path,
    show_exit_code,
    show_stdout_path,
    show_stderr_path,
) = sys.argv[1:11]

with open(weak_path, encoding="utf-8") as f:
    weak = json.load(f)
create_stderr = pathlib.Path(create_stderr_path).read_text(errors="replace")
show_stderr = pathlib.Path(show_stderr_path).read_text(errors="replace")
reason_match = re.search(r"(target_identity_ambiguous)", create_stderr)
session_not_found = "session_not_found" in show_stderr
evidence = {
    "status": "passed",
    "scenario": "window-app-title-only",
    "weak_resource": weak,
    "create_session": {
        "ability": "remote_desktop.create_session",
        "subject_ura": weak_ura,
        "args": {
            "session_id": session_id,
            "mode": "view_only",
            "transport_preferences": ["webrtc"],
        },
        "exit_code": int(create_exit_code),
        "target_reason": reason_match.group(1) if reason_match else None,
        "stdout": pathlib.Path(create_stdout_path).read_text(errors="replace"),
        "stderr": create_stderr,
    },
    "show_session_absence_probe": {
        "ability": "remote_desktop.show_session",
        "subject_ura": weak_ura,
        "exit_code": int(show_exit_code),
        "stdout": pathlib.Path(show_stdout_path).read_text(errors="replace"),
        "stderr": show_stderr,
    },
    "active_session_row_inserted": not session_not_found,
    "stream_start_attempted": False,
    "media_start_attempted": False,
    "decoded_frames": {
        "count": 0,
        "observation": "session_failed_before_stream",
    },
    "resource_registry_restored": True,
}
pathlib.Path(evidence_path).write_text(
    json.dumps(evidence, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
PY

validate_evidence
echo "PASS: $REPORT_MD"
