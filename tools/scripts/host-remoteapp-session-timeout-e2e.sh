#!/usr/bin/env bash
# Host-side RemoteApp session timeout E2E.
#
# Boundary:
# - This script proves a public CLI-created RemoteApp session reaches a
#   terminal timeout state through the daemon session lifecycle.
# - It observes timeout through `remote_desktop.show_session` and verifies
#   idempotent `remote_desktop.end_session` after timeout.
# - It does not prove reconnect, crash recovery, cross-device network paths, or
#   successful interactive input injection.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
BUNDLED_SENTINEL_FIXTURE="$REPO_ROOT/tools/scripts/host-remoteapp-sentinel-fixture.sh"
source "$SELF_DIR/remoteapp-lifecycle-harness-lib.sh"

MODE=run
TARGET_KIND=window
OUT_DIR=""
SENTINEL_FIXTURE=0
SENTINEL_FIXTURE_CMD="${EASYNET_REMOTEAPP_SENTINEL_FIXTURE_CMD:-}"
LEASE_TTL_MS="${EASYNET_REMOTEAPP_TIMEOUT_E2E_LEASE_TTL_MS:-25}"
WAIT_AFTER_LEASE_MS="${EASYNET_REMOTEAPP_TIMEOUT_E2E_WAIT_AFTER_LEASE_MS:-250}"

usage() {
  cat <<'USAGE'
Usage:
  host-remoteapp-session-timeout-e2e.sh --run --sentinel-fixture
  host-remoteapp-session-timeout-e2e.sh --self-test

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
  --lease-ttl-ms MS     Short lease TTL for timeout proof. Default: 25.
  --wait-after-lease-ms MS
                        Wait duration before show_session timeout observation.
                        Default: 250.
  --out-dir DIR         Report directory. Defaults under target/e2e.

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
        *) echo "invalid timeout E2E target kind: $2" >&2; exit 64 ;;
      esac
      shift 2
      ;;
    --sentinel-fixture) SENTINEL_FIXTURE=1; shift ;;
    --sentinel-fixture-cmd)
      SENTINEL_FIXTURE=1
      SENTINEL_FIXTURE_CMD="${2:?missing value for --sentinel-fixture-cmd}"
      shift 2
      ;;
    --lease-ttl-ms) LEASE_TTL_MS="${2:?missing value for --lease-ttl-ms}"; shift 2 ;;
    --wait-after-lease-ms) WAIT_AFTER_LEASE_MS="${2:?missing value for --wait-after-lease-ms}"; shift 2 ;;
    --out-dir) OUT_DIR="${2:?missing value for --out-dir}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

TIMESTAMP="$(date -u +%Y%m%d-%H%M%S)"
if [[ -z "$OUT_DIR" ]]; then
  OUT_DIR="$REPO_ROOT/target/e2e/host-remoteapp-session-timeout/$TIMESTAMP-$TARGET_KIND-$$"
fi
mkdir -p "$OUT_DIR"

EVIDENCE_JSON="$OUT_DIR/evidence.json"
REPORT_JSON="$OUT_DIR/report.json"
REPORT_MD="$OUT_DIR/report.md"
LIVE_INVENTORY_JSON="$OUT_DIR/live-inventory.json"
SELECTED_RESOURCE_JSON="$OUT_DIR/selected-resource.json"
SENTINEL_MANIFEST_JSON="$OUT_DIR/sentinel-manifest.json"
CREATE_SESSION_JSON="$OUT_DIR/create-session.json"
SHOW_AFTER_TIMEOUT_JSON="$OUT_DIR/show-after-timeout.json"
END_AFTER_TIMEOUT_JSON="$OUT_DIR/end-after-timeout.json"
ABILITY_CATALOG_JSON="$OUT_DIR/ability-catalog.json"
SESSION_ID="rd-session-timeout-e2e-$$"

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

json_first_value_to_file() {
  local source_path="$1"
  local target_path="$2"
  python3 - "$source_path" "$target_path" <<'PY'
import json
import pathlib
import sys

source = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
value, _ = json.JSONDecoder().raw_decode(source.lstrip())
pathlib.Path(sys.argv[2]).write_text(
    json.dumps(value, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
PY
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

selected = evidence.get("selected_resource")
create = evidence.get("create_session")
show = evidence.get("show_after_timeout")
ended = evidence.get("end_after_timeout")
resource_ura = selected.get("resource_ura") if isinstance(selected, dict) else None

require(evidence.get("status") == "passed", "evidence.status must be passed")
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
require(isinstance(evidence.get("lease_ttl_ms"), int) and evidence["lease_ttl_ms"] >= 1,
        "lease_ttl_ms must be a positive integer")
require(isinstance(evidence.get("wait_after_lease_ms"), int)
        and evidence["wait_after_lease_ms"] >= evidence["lease_ttl_ms"],
        "wait_after_lease_ms must cover the requested lease")

require(isinstance(create, dict), "create_session evidence must be recorded")
require(get("create_session.ability") == "remote_desktop.create_session",
        "create_session ability must be remote_desktop.create_session")
require(get("create_session.subject_ura") == resource_ura,
        "create_session subject must equal selected Resource URA")
require(get("create_session.exit_code") == 0, "create_session must succeed")
args = get("create_session.args")
require(isinstance(args, dict), "create_session args must be recorded")
if isinstance(args, dict):
    require(args.get("lease_ttl_ms") == evidence.get("lease_ttl_ms"),
            "create_session args must preserve the short lease TTL")
    require(not contains_subject_arg(args),
            "create_session args must not carry subject identity")

require(isinstance(show, dict), "show_after_timeout evidence must be recorded")
require(get("show_after_timeout.ability") == "remote_desktop.show_session",
        "show_after_timeout ability must be remote_desktop.show_session")
require(get("show_after_timeout.subject_ura") == resource_ura,
        "show_after_timeout subject must equal selected Resource URA")
require(get("show_after_timeout.exit_code") == 0, "show_after_timeout must succeed")
require(get("show_after_timeout.session.state") == "closed",
        "show_after_timeout session must be closed")
require(get("show_after_timeout.session.end_reason") == "session_expired",
        "show_after_timeout end_reason must be session_expired")
require(get("show_after_timeout.session.terminal_receipt.reason_code") == "session_expired",
        "show_after_timeout terminal_receipt.reason_code must be session_expired")
require(get("show_after_timeout.session.terminal_receipt.terminal") is True,
        "show_after_timeout terminal_receipt must be terminal")
require(get("show_after_timeout.session.terminal_receipt.session_id")
        == get("create_session.session.session_id"),
        "terminal receipt must bind the created session id")

require(isinstance(ended, dict), "end_after_timeout evidence must be recorded")
require(get("end_after_timeout.ability") == "remote_desktop.end_session",
        "end_after_timeout ability must be remote_desktop.end_session")
require(get("end_after_timeout.subject_ura") == resource_ura,
        "end_after_timeout subject must equal selected Resource URA")
require(get("end_after_timeout.exit_code") == 0, "end_after_timeout must succeed")
require(get("end_after_timeout.session.already_ended") is True,
        "end_after_timeout must be idempotent after timeout")
require(get("end_after_timeout.session.end_reason") == "session_expired",
        "end_after_timeout must preserve session_expired reason")
require(get("end_after_timeout.session.terminal_receipt")
        == get("show_after_timeout.session.terminal_receipt"),
        "end_after_timeout must preserve the original timeout terminal receipt")

report = {
    "script": "tools/scripts/host-remoteapp-session-timeout-e2e.sh",
    "status": "failed" if errors else "passed",
    "errors": errors,
    "evidence_json": evidence_path,
    "target_kind": evidence.get("target_kind"),
    "selected_resource_ura": resource_ura,
    "session_id": get("create_session.session.session_id"),
    "lease_ttl_ms": evidence.get("lease_ttl_ms"),
    "timeout_reason": get("show_after_timeout.session.terminal_receipt.reason_code"),
    "idempotent_end": get("end_after_timeout.session.already_ended"),
    "lifecycle_summary": {
        "kind": "session_timeout",
        "terminal_state": get("show_after_timeout.session.state"),
        "terminal_reason": get("show_after_timeout.session.end_reason"),
        "terminal_receipt_visible": get("show_after_timeout.session.terminal_receipt.terminal") is True,
        "terminal_receipt_session_bound": (
            get("show_after_timeout.session.terminal_receipt.session_id")
            == get("create_session.session.session_id")
        ),
        "idempotent_end": get("end_after_timeout.session.already_ended") is True,
        "idempotent_end_preserved_receipt": (
            get("end_after_timeout.session.terminal_receipt")
            == get("show_after_timeout.session.terminal_receipt")
        ),
        "selected_from_live_refresh": evidence.get("selected_from_live_refresh") is True,
    },
}
pathlib.Path(report_path).write_text(
    json.dumps(report, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
with open(md_path, "w", encoding="utf-8") as f:
    f.write("# RemoteApp session timeout E2E report\n\n")
    f.write(f"- Status: `{report['status']}`\n")
    f.write(f"- Target kind: `{report['target_kind']}`\n")
    f.write(f"- Selected Resource URA: `{report['selected_resource_ura']}`\n")
    f.write(f"- Session id: `{report['session_id']}`\n")
    f.write(f"- Lease TTL ms: `{report['lease_ttl_ms']}`\n")
    f.write(f"- Timeout reason: `{report['timeout_reason']}`\n")
    f.write(f"- Idempotent end after timeout: `{report['idempotent_end']}`\n")
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
subject = "easynet:///r/localhost/resource/device.dev/streams/window.timeout"
terminal_receipt = {
    "receipt_type": "remote_desktop.session_terminal",
    "session_id": "rd-session-timeout-e2e-self-test",
    "reason_code": "session_expired",
    "terminal": True,
    "terminal_event_id": "event-timeout",
    "terminal_event_sequence": 3,
}
session = {
    "session_id": "rd-session-timeout-e2e-self-test",
    "subject_ura": subject,
    "state": "closed",
    "end_reason": "session_expired",
    "terminal_receipt": terminal_receipt,
}
evidence = {
    "status": "passed",
    "target_kind": "window",
    "lease_ttl_ms": 25,
    "wait_after_lease_ms": 250,
    "selected_from_live_refresh": True,
    "selected_resource": {
        "resource_ura": subject,
        "type": "window",
        "display_name": "EasyNet selected timeout sentinel fixture",
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
            "session_id": "rd-session-timeout-e2e-self-test",
            "mode": "view_only",
            "transport_preferences": ["webrtc"],
            "lease_ttl_ms": 25,
        },
        "session": {
            "session_id": "rd-session-timeout-e2e-self-test",
            "subject_ura": subject,
            "state": "negotiating",
            "terminal_receipt": None,
        },
    },
    "show_after_timeout": {
        "ability": "remote_desktop.show_session",
        "subject_ura": subject,
        "exit_code": 0,
        "session": session,
    },
    "end_after_timeout": {
        "ability": "remote_desktop.end_session",
        "subject_ura": subject,
        "exit_code": 0,
        "session": {**session, "already_ended": True},
    },
}
path.write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
  validate_evidence
  echo "host-remoteapp-session-timeout-e2e self-test ok"
  exit 0
fi

need_cmd python3
case "$LEASE_TTL_MS" in
  ''|*[!0-9]*) die "--lease-ttl-ms must be a positive integer" ;;
esac
case "$WAIT_AFTER_LEASE_MS" in
  ''|*[!0-9]*) die "--wait-after-lease-ms must be a positive integer" ;;
esac
(( LEASE_TTL_MS >= 1 )) || die "--lease-ttl-ms must be >= 1"
(( WAIT_AFTER_LEASE_MS >= LEASE_TTL_MS )) || die "--wait-after-lease-ms must be >= lease ttl"

if [[ "$TARGET_KIND" != "display" ]]; then
  [[ "$SENTINEL_FIXTURE" == "1" ]] || die "--sentinel-fixture is required for live window/application timeout E2E"
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
  --lease-ttl-ms "$LEASE_TTL_MS" \
  --format json >"$CREATE_SESSION_JSON"

python3 - "$WAIT_AFTER_LEASE_MS" <<'PY'
import sys
import time
time.sleep(int(sys.argv[1]) / 1000)
PY

run_easynet ability show-remote-desktop-session \
  --session-json "$CREATE_SESSION_JSON" \
  --format json >"$SHOW_AFTER_TIMEOUT_JSON"

SESSION_TOKEN="$(python3 - "$CREATE_SESSION_JSON" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as f:
    response = json.load(f)
token = response.get("session", {}).get("session_token")
if not isinstance(token, str) or not token:
    raise SystemExit("create_session response missing session.session_token")
print(token)
PY
)"
SESSION_CAUSAL_CONTEXT_JSON="$(remoteapp_session_approval_causal_context_json "$CREATE_SESSION_JSON")"
END_ARGS="$(python3 - "$SESSION_ID" "$SESSION_TOKEN" <<'PY'
import json
import sys
print(json.dumps({
    "session_id": sys.argv[1],
    "session_token": sys.argv[2],
    "reason": "post_timeout_audit",
}, separators=(",", ":")))
PY
)"
END_NONCE_HEX="$(python3 - <<'PY'
import secrets
print(secrets.token_hex(16))
PY
)"
END_RAW_JSON="$OUT_DIR/end-after-timeout-raw.txt"
run_easynet ability list --format json >"$ABILITY_CATALOG_JSON"
END_SESSION_ABILITY_URA="$(remoteapp_resolve_rpc_ability_ura "$ABILITY_CATALOG_JSON" remote_desktop.end_session)"
run_easynet ability invoke "$END_SESSION_ABILITY_URA" \
  --subject "$SELECTED_RESOURCE_URA" \
  --nonce-hex "$END_NONCE_HEX" \
  --causal-context-json "$SESSION_CAUSAL_CONTEXT_JSON" \
  --args "$END_ARGS" >"$END_RAW_JSON"
json_first_value_to_file "$END_RAW_JSON" "$END_AFTER_TIMEOUT_JSON"

python3 - "$EVIDENCE_JSON" "$TARGET_KIND" "$LEASE_TTL_MS" "$WAIT_AFTER_LEASE_MS" "$SENTINEL_MANIFEST_JSON" "$LIVE_INVENTORY_JSON" "$SELECTED_RESOURCE_JSON" "$CREATE_SESSION_JSON" "$SHOW_AFTER_TIMEOUT_JSON" "$END_AFTER_TIMEOUT_JSON" <<'PY'
import json
import pathlib
import sys

(
    evidence_path,
    target_kind,
    lease_ttl_ms,
    wait_after_lease_ms,
    fixture_manifest_path,
    live_inventory_path,
    selected_resource_path,
    create_session_path,
    show_after_timeout_path,
    end_after_timeout_path,
) = sys.argv[1:11]

def load(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)

fixture = load(fixture_manifest_path)
live_inventory = load(live_inventory_path)
selected = load(selected_resource_path)
create_response = load(create_session_path)
show_response = load(show_after_timeout_path)
end_response = load(end_after_timeout_path)
create_session = create_response.get("session")
invocation = create_response.get("invocation")
if not isinstance(create_session, dict):
    raise SystemExit("create-remote-desktop-session response missing session object")
if not isinstance(invocation, dict):
    raise SystemExit("create-remote-desktop-session response missing verified invocation metadata")
if not isinstance(show_response, dict):
    raise SystemExit("show-remote-desktop-session response missing session object")
if not isinstance(end_response, dict):
    raise SystemExit("end_session response missing session object")

evidence = {
    "status": "passed",
    "target_kind": target_kind,
    "lease_ttl_ms": int(lease_ttl_ms),
    "wait_after_lease_ms": int(wait_after_lease_ms),
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
    "show_after_timeout": {
        "ability": "remote_desktop.show_session",
        "subject_ura": selected.get("resource_ura"),
        "exit_code": 0,
        "session": show_response,
    },
    "end_after_timeout": {
        "ability": "remote_desktop.end_session",
        "subject_ura": selected.get("resource_ura"),
        "exit_code": 0,
        "session": end_response,
    },
}
pathlib.Path(evidence_path).write_text(
    json.dumps(evidence, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
PY

validate_evidence
echo "host-remoteapp-session-timeout-e2e ok: $REPORT_MD"
