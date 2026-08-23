#!/usr/bin/env bash
# Host-side RemoteApp permission-revoke E2E.
#
# Boundary:
# - Live mode creates a public RemoteApp session, then waits for a real platform
#   permission revocation to terminate that session as target_permission_revoked.
# - It does not simulate or invoke a debug revoke path. The terminal state must
#   be observed through public remote_desktop.show_session.
# - Self-test validates the evidence contract only; it is not product evidence.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
BUNDLED_SENTINEL_FIXTURE="$REPO_ROOT/tools/scripts/host-remoteapp-sentinel-fixture.sh"

MODE=run
TARGET_KIND=window
OUT_DIR=""
SENTINEL_FIXTURE=0
SENTINEL_FIXTURE_CMD="${EASYNET_REMOTEAPP_SENTINEL_FIXTURE_CMD:-}"
WAIT_FOR_REVOKE_MS="${EASYNET_REMOTEAPP_REVOKE_E2E_WAIT_MS:-45000}"
POLL_INTERVAL_MS="${EASYNET_REMOTEAPP_REVOKE_E2E_POLL_MS:-1000}"

usage() {
  cat <<'USAGE'
Usage:
  host-remoteapp-permission-revoke-e2e.sh --run --sentinel-fixture
  host-remoteapp-permission-revoke-e2e.sh --self-test

Options:
  --run                 Execute against the local EasyNet daemon.
  --self-test           Validate the harness against synthetic positive evidence.
  --target-kind KIND    display, window, or application. Default: window.
  --sentinel-fixture    Launch the bundled native AppKit selected/unrelated
                        fixture and select the known target. Required for live
                        window/application probes.
  --sentinel-fixture-cmd CMD
                        Override fixture command. Receives
                        EASYNET_REMOTEAPP_SENTINEL_FIXTURE_DIR and must write
                        env.sh plus cleanup.sh.
  --wait-for-revoke-ms MS
                        How long to wait for the operator/platform to revoke
                        host permission. Default: 45000.
  --poll-interval-ms MS Public show_session polling interval. Default: 1000.
  --out-dir DIR         Report directory. Defaults under target/e2e.

Live-mode operator action:
  After session creation, revoke the host Screen Recording permission for the
  EasyNet daemon process while this script is waiting. The script passes only
  if the daemon session closes with target_permission_revoked.

Environment:
  EASYNET_REMOTEAPP_EASYNET_BIN
                        Optional easynet binary override.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --run) MODE=run; shift ;;
    --self-test) MODE=self-test; shift ;;
    --target-kind)
      case "${2:?missing value for --target-kind}" in
        display|window|application) TARGET_KIND="$2" ;;
        *) echo "invalid permission revoke E2E target kind: $2" >&2; exit 64 ;;
      esac
      shift 2
      ;;
    --sentinel-fixture) SENTINEL_FIXTURE=1; shift ;;
    --sentinel-fixture-cmd)
      SENTINEL_FIXTURE=1
      SENTINEL_FIXTURE_CMD="${2:?missing value for --sentinel-fixture-cmd}"
      shift 2
      ;;
    --wait-for-revoke-ms) WAIT_FOR_REVOKE_MS="${2:?missing value for --wait-for-revoke-ms}"; shift 2 ;;
    --poll-interval-ms) POLL_INTERVAL_MS="${2:?missing value for --poll-interval-ms}"; shift 2 ;;
    --out-dir) OUT_DIR="${2:?missing value for --out-dir}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

TIMESTAMP="$(date -u +%Y%m%d-%H%M%S)"
if [[ -z "$OUT_DIR" ]]; then
  OUT_DIR="$REPO_ROOT/target/e2e/host-remoteapp-permission-revoke/$TIMESTAMP-$TARGET_KIND-$$"
fi
mkdir -p "$OUT_DIR"

EVIDENCE_JSON="$OUT_DIR/evidence.json"
REPORT_JSON="$OUT_DIR/report.json"
REPORT_MD="$OUT_DIR/report.md"
LIVE_INVENTORY_JSON="$OUT_DIR/live-inventory.json"
SELECTED_RESOURCE_JSON="$OUT_DIR/selected-resource.json"
SENTINEL_MANIFEST_JSON="$OUT_DIR/sentinel-manifest.json"
CREATE_SESSION_JSON="$OUT_DIR/create-session.json"
SHOW_AFTER_REVOKE_JSON="$OUT_DIR/show-after-revoke.json"
SESSION_ID="rd-session-permission-revoke-e2e-$$"

die() {
  echo "[FAIL] $*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "missing command: $1"
}

run_easynet() {
  if [[ -n "${EASYNET_REMOTEAPP_EASYNET_BIN:-}" ]]; then
    "$EASYNET_REMOTEAPP_EASYNET_BIN" "$@"
  elif [[ -x "$REPO_ROOT/target/debug/easynet" ]]; then
    "$REPO_ROOT/target/debug/easynet" "$@"
  else
    need_cmd cargo
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

def event_index(events, event_type):
    if not isinstance(events, list):
        return None
    for index, event in enumerate(events):
        if isinstance(event, dict) and event.get("event_type") == event_type:
            return index
    return None

selected = evidence.get("selected_resource")
create = evidence.get("create_session")
show = evidence.get("show_after_revoke")
resource_ura = selected.get("resource_ura") if isinstance(selected, dict) else None
events = get("show_after_revoke.session.events", [])
permission_event = event_index(events, "TARGET_PERMISSION_REVOKED")
media_lost_event = event_index(events, "MEDIA_SOURCE_LOST")
closed_event = event_index(events, "SESSION_CLOSED")

require(evidence.get("status") == "passed", "evidence.status must be passed")
require(evidence.get("proof_mode") == "real_platform_permission_revoke",
        "proof_mode must require real platform permission revoke")
require(evidence.get("operator_revoke_required") is True,
        "live evidence must require operator/platform permission revocation")
require(evidence.get("target_kind") in {"display", "window", "application"},
        "target_kind must be display, window, or application")
require(isinstance(resource_ura, str) and resource_ura.startswith("easynet:///"),
        "selected resource_ura must be a canonical EasyNet URA")
require(get("selected_resource.metadata.availability") == "available",
        "selected target must be available")
require(get("selected_resource.metadata.discovery_source") == "resource.refresh_remote_targets",
        "selected target must come from resource.refresh_remote_targets")
require(evidence.get("selected_from_live_refresh") is True,
        "target must be selected from a live refresh")

require(isinstance(create, dict), "create_session evidence must be recorded")
require(get("create_session.ability") == "remote_desktop.create_session",
        "create_session ability must be remote_desktop.create_session")
require(get("create_session.subject_ura") == resource_ura,
        "create_session subject must equal selected Resource URA")
require(get("create_session.exit_code") == 0, "create_session must succeed")
args = get("create_session.args")
require(isinstance(args, dict), "create_session args must be recorded")
if isinstance(args, dict):
    require(not contains_subject_arg(args),
            "create_session args must not carry subject identity")

require(isinstance(show, dict), "show_after_revoke evidence must be recorded")
require(get("show_after_revoke.ability") == "remote_desktop.show_session",
        "show_after_revoke ability must be remote_desktop.show_session")
require(get("show_after_revoke.subject_ura") == resource_ura,
        "show_after_revoke subject must equal selected Resource URA")
require(get("show_after_revoke.exit_code") == 0, "show_after_revoke must succeed")
require(get("show_after_revoke.session.state") == "closed",
        "show_after_revoke session must be closed")
require(get("show_after_revoke.session.end_reason") == "target_permission_revoked",
        "show_after_revoke end_reason must be target_permission_revoked")
require(get("show_after_revoke.session.consent_phase") == "revoked",
        "show_after_revoke consent_phase must be revoked")
require(get("show_after_revoke.session.consent.phase") == "revoked",
        "show_after_revoke consent projection must be revoked")
require(get("show_after_revoke.session.terminal_receipt.reason_code")
        == "target_permission_revoked",
        "show_after_revoke terminal_receipt.reason_code must be target_permission_revoked")
require(get("show_after_revoke.session.terminal_receipt.terminal") is True,
        "show_after_revoke terminal_receipt must be terminal")
require(get("show_after_revoke.session.terminal_receipt.session_id")
        == get("create_session.session.session_id"),
        "permission revoke terminal receipt must bind the created session id")
require(permission_event is not None,
        "show_after_revoke events must include TARGET_PERMISSION_REVOKED")
require(media_lost_event is not None,
        "show_after_revoke events must include MEDIA_SOURCE_LOST")
require(closed_event is not None,
        "show_after_revoke events must include SESSION_CLOSED")
if permission_event is not None and media_lost_event is not None:
    require(permission_event < media_lost_event,
            "TARGET_PERMISSION_REVOKED must precede MEDIA_SOURCE_LOST")
if media_lost_event is not None and closed_event is not None:
    require(media_lost_event < closed_event,
            "MEDIA_SOURCE_LOST must precede SESSION_CLOSED")

report = {
    "script": "tools/scripts/host-remoteapp-permission-revoke-e2e.sh",
    "status": "failed" if errors else "passed",
    "errors": errors,
    "evidence_json": evidence_path,
    "target_kind": evidence.get("target_kind"),
    "selected_resource_ura": resource_ura,
    "session_id": get("create_session.session.session_id"),
    "terminal_reason": get("show_after_revoke.session.terminal_receipt.reason_code"),
    "proof_mode": evidence.get("proof_mode"),
}
pathlib.Path(report_path).write_text(
    json.dumps(report, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
with open(md_path, "w", encoding="utf-8") as f:
    f.write("# RemoteApp permission revoke E2E report\n\n")
    f.write(f"- Status: `{report['status']}`\n")
    f.write(f"- Target kind: `{report['target_kind']}`\n")
    f.write(f"- Selected Resource URA: `{report['selected_resource_ura']}`\n")
    f.write(f"- Session id: `{report['session_id']}`\n")
    f.write(f"- Terminal reason: `{report['terminal_reason']}`\n")
    f.write(f"- Proof mode: `{report['proof_mode']}`\n")
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
subject = "easynet:///r/localhost/resource/device.dev/streams/window.revoked"
events = [
    {"event_type": "SESSION_CREATED", "sequence": 1},
    {"event_type": "TARGET_PERMISSION_REVOKED", "sequence": 2},
    {"event_type": "MEDIA_SOURCE_LOST", "sequence": 3},
    {"event_type": "SESSION_CLOSED", "sequence": 4},
]
terminal_receipt = {
    "receipt_type": "remoteapp.session.terminal.v1",
    "session_id": "rd-session-permission-revoke-e2e-self-test",
    "reason_code": "target_permission_revoked",
    "terminal": True,
    "terminal_event_id": "event-close",
    "terminal_event_sequence": 4,
}
session = {
    "session_id": "rd-session-permission-revoke-e2e-self-test",
    "subject_ura": subject,
    "state": "closed",
    "consent_phase": "revoked",
    "consent": {"phase": "revoked"},
    "end_reason": "target_permission_revoked",
    "terminal_receipt": terminal_receipt,
    "events": events,
}
evidence = {
    "status": "passed",
    "proof_mode": "real_platform_permission_revoke",
    "operator_revoke_required": True,
    "target_kind": "window",
    "selected_from_live_refresh": True,
    "selected_resource": {
        "resource_ura": subject,
        "type": "window",
        "display_name": "EasyNet selected permission revoke sentinel fixture",
        "metadata": {
            "availability": "available",
            "discovery_source": "resource.refresh_remote_targets",
        },
    },
    "create_session": {
        "ability": "remote_desktop.create_session",
        "subject_ura": subject,
        "exit_code": 0,
        "args": {
            "session_id": "rd-session-permission-revoke-e2e-self-test",
            "mode": "view_only",
            "transport_preferences": ["webrtc"],
        },
        "session": {
            "session_id": "rd-session-permission-revoke-e2e-self-test",
            "subject_ura": subject,
            "state": "negotiating",
            "consent_phase": "active",
            "terminal_receipt": None,
        },
    },
    "show_after_revoke": {
        "ability": "remote_desktop.show_session",
        "subject_ura": subject,
        "exit_code": 0,
        "session": session,
    },
}
path.write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
  validate_evidence
  echo "host-remoteapp-permission-revoke-e2e self-test ok"
  exit 0
fi

need_cmd python3
case "$WAIT_FOR_REVOKE_MS" in
  ''|*[!0-9]*) die "--wait-for-revoke-ms must be a positive integer" ;;
esac
case "$POLL_INTERVAL_MS" in
  ''|*[!0-9]*) die "--poll-interval-ms must be a positive integer" ;;
esac
(( WAIT_FOR_REVOKE_MS >= 1000 )) || die "--wait-for-revoke-ms must be >= 1000"
(( POLL_INTERVAL_MS >= 100 )) || die "--poll-interval-ms must be >= 100"

if [[ "$TARGET_KIND" != "display" ]]; then
  [[ "$SENTINEL_FIXTURE" == "1" ]] || die "--sentinel-fixture is required for live window/application permission revoke E2E"
fi
if [[ "$SENTINEL_FIXTURE" == "1" ]]; then
  if [[ -z "$SENTINEL_FIXTURE_CMD" ]]; then
    [[ -x "$BUNDLED_SENTINEL_FIXTURE" ]] || die "missing bundled sentinel fixture: $BUNDLED_SENTINEL_FIXTURE"
    SENTINEL_FIXTURE_CMD="$BUNDLED_SENTINEL_FIXTURE --target-kind $TARGET_KIND"
  fi
  SENTINEL_FIXTURE_DIR="$OUT_DIR/sentinel-fixture"
  mkdir -p "$SENTINEL_FIXTURE_DIR"
  export EASYNET_REMOTEAPP_SENTINEL_FIXTURE_DIR="$SENTINEL_FIXTURE_DIR"
  trap '[[ -x "$SENTINEL_FIXTURE_DIR/cleanup.sh" ]] && "$SENTINEL_FIXTURE_DIR/cleanup.sh" >/dev/null 2>&1 || true' EXIT
  bash -lc "$SENTINEL_FIXTURE_CMD"
  [[ -f "$SENTINEL_FIXTURE_DIR/env.sh" ]] || die "sentinel fixture did not write env.sh"
  source "$SENTINEL_FIXTURE_DIR/env.sh"
  [[ -n "${EASYNET_REMOTEAPP_TARGET_PID:-}" ]] || die "sentinel fixture did not export selected target pid"
  [[ -n "${EASYNET_REMOTEAPP_TARGET_HINT:-}" ]] || die "sentinel fixture did not export selected target hint"
  [[ -f "${EASYNET_REMOTEAPP_SENTINEL_FIXTURE_MANIFEST:-}" ]] || die "sentinel fixture did not export manifest path"
  cp "$EASYNET_REMOTEAPP_SENTINEL_FIXTURE_MANIFEST" "$SENTINEL_MANIFEST_JSON"
else
  printf '{"kind":"none","reason":"display target does not require sentinel fixture"}\n' >"$SENTINEL_MANIFEST_JSON"
fi

run_easynet ability refresh-remote-targets --type "$TARGET_KIND" --format json >"$LIVE_INVENTORY_JSON"
python3 - "$LIVE_INVENTORY_JSON" "$SELECTED_RESOURCE_JSON" "$TARGET_KIND" "${EASYNET_REMOTEAPP_TARGET_PID:-}" "${EASYNET_REMOTEAPP_TARGET_HINT:-}" <<'PY'
import json
import sys

inventory_path, selected_path, target_kind, target_pid, target_hint = sys.argv[1:6]
with open(inventory_path, encoding="utf-8") as f:
    inventory = json.load(f)
resources = inventory.get("resources")
if not isinstance(resources, list):
    raise SystemExit("resource.refresh_remote_targets response missing resources array")

def metadata(resource):
    return resource.get("metadata") if isinstance(resource.get("metadata"), dict) else {}

def pid_matches(resource):
    if not target_pid:
        return True
    meta = metadata(resource)
    values = [meta.get("pid"), meta.get("primary_pid")]
    return any(str(value) == str(target_pid) for value in values if value is not None)

def text_matches(resource):
    if not target_hint:
        return True
    meta = metadata(resource)
    fields = [
        resource.get("display_name"),
        meta.get("title"),
        meta.get("primary_title"),
        meta.get("app_name"),
        meta.get("bundle_id"),
        meta.get("app_identity"),
    ]
    return any(str(value) == target_hint for value in fields if value is not None)

candidates = [
    resource for resource in resources
    if resource.get("type") == target_kind
    and metadata(resource).get("availability") == "available"
    and pid_matches(resource)
    and text_matches(resource)
]
if not candidates:
    raise SystemExit(f"no available {target_kind} target resolved from live refresh")
if target_hint and len(candidates) != 1:
    raise SystemExit(f"known {target_kind} sentinel target must resolve exactly once; got {len(candidates)}")
with open(selected_path, "w", encoding="utf-8") as f:
    json.dump(candidates[0], f, indent=2, sort_keys=True)
    f.write("\n")
PY

SELECTED_RESOURCE_URA="$(python3 - "$SELECTED_RESOURCE_JSON" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as f:
    print(json.load(f)["resource_ura"])
PY
)"

run_easynet ability create-remote-desktop-session \
  --subject "$SELECTED_RESOURCE_URA" \
  --session-id "$SESSION_ID" \
  --mode view_only \
  --transport webrtc \
  --format json >"$CREATE_SESSION_JSON"

echo "[INFO] RemoteApp session created: $SESSION_ID" >&2
echo "[INFO] Revoke the host Screen Recording permission for the EasyNet daemon now." >&2
echo "[INFO] Waiting up to ${WAIT_FOR_REVOKE_MS}ms for target_permission_revoked via public show_session..." >&2

deadline_ms="$(python3 - "$WAIT_FOR_REVOKE_MS" <<'PY'
import sys
import time
print(int(time.time() * 1000) + int(sys.argv[1]))
PY
)"
while true; do
  run_easynet ability show-remote-desktop-session \
    --session-json "$CREATE_SESSION_JSON" \
    --format json >"$SHOW_AFTER_REVOKE_JSON"
  status="$(python3 - "$SHOW_AFTER_REVOKE_JSON" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as f:
    value = json.load(f)
print(f"{value.get('state')}:{value.get('end_reason')}")
PY
)"
  if [[ "$status" == "closed:target_permission_revoked" ]]; then
    break
  fi
  if [[ "$status" == closed:* ]]; then
    die "session closed with unexpected terminal status: $status"
  fi
  now_ms="$(python3 - <<'PY'
import time
print(int(time.time() * 1000))
PY
)"
  if (( now_ms >= deadline_ms )); then
    echo "[INFO] Permission revoke was not observed; ending session for cleanup." >&2
    session_token="$(python3 - "$CREATE_SESSION_JSON" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as f:
    print(json.load(f).get("session", {}).get("session_token", ""))
PY
)"
    if [[ -n "$session_token" ]]; then
      cleanup_args="$(python3 - "$SESSION_ID" "$session_token" <<'PY'
import json
import sys
print(json.dumps({
    "session_id": sys.argv[1],
    "session_token": sys.argv[2],
    "reason": "permission_revoke_e2e_cleanup",
}, separators=(",", ":")))
PY
)"
      cleanup_nonce="$(python3 - <<'PY'
import secrets
print(secrets.token_hex(16))
PY
)"
      run_easynet ability invoke remote_desktop.end_session \
        --subject "$SELECTED_RESOURCE_URA" \
        --nonce-hex "$cleanup_nonce" \
        --causal-root \
        --args "$cleanup_args" >/dev/null 2>&1 || true
    fi
    die "target_permission_revoked was not observed before timeout"
  fi
  python3 - "$POLL_INTERVAL_MS" <<'PY'
import sys
import time
time.sleep(int(sys.argv[1]) / 1000)
PY
done

python3 - "$EVIDENCE_JSON" "$TARGET_KIND" "$SENTINEL_MANIFEST_JSON" "$LIVE_INVENTORY_JSON" "$SELECTED_RESOURCE_JSON" "$CREATE_SESSION_JSON" "$SHOW_AFTER_REVOKE_JSON" <<'PY'
import json
import pathlib
import sys

(
    evidence_path,
    target_kind,
    fixture_manifest_path,
    live_inventory_path,
    selected_resource_path,
    create_session_path,
    show_after_revoke_path,
) = sys.argv[1:8]

def load(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)

fixture = load(fixture_manifest_path)
live_inventory = load(live_inventory_path)
selected = load(selected_resource_path)
create_response = load(create_session_path)
show_response = load(show_after_revoke_path)
create_session = create_response.get("session")
invocation = create_response.get("invocation")
if not isinstance(create_session, dict):
    raise SystemExit("create-remote-desktop-session response missing session object")
if not isinstance(invocation, dict):
    raise SystemExit("create-remote-desktop-session response missing verified invocation metadata")
if not isinstance(show_response, dict):
    raise SystemExit("show-remote-desktop-session response missing session object")

evidence = {
    "status": "passed",
    "proof_mode": "real_platform_permission_revoke",
    "operator_revoke_required": True,
    "target_kind": target_kind,
    "selected_from_live_refresh": True,
    "sentinel_fixture": fixture,
    "live_inventory": {
        "ability": "resource.refresh_remote_targets",
        "target_kind": target_kind,
        "observed_at_ms": live_inventory.get("observed_at_ms"),
        "freshness_ttl_ms": live_inventory.get("freshness_ttl_ms"),
        "resource_count": len(live_inventory.get("resources", []))
        if isinstance(live_inventory.get("resources"), list)
        else None,
    },
    "selected_resource": selected,
    "create_session": {
        "ability": invocation.get("ability"),
        "subject_ura": invocation.get("subject_ura"),
        "exit_code": 0,
        "args": invocation.get("args"),
        "session": create_session,
    },
    "show_after_revoke": {
        "ability": "remote_desktop.show_session",
        "subject_ura": selected.get("resource_ura"),
        "exit_code": 0,
        "session": show_response,
    },
}
pathlib.Path(evidence_path).write_text(
    json.dumps(evidence, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
PY

validate_evidence
echo "host-remoteapp-permission-revoke-e2e ok: $REPORT_MD"
