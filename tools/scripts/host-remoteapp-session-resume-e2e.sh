#!/usr/bin/env bash
# Host-side RemoteApp session resume/lease-refresh E2E.
#
# Boundary:
# - This script proves a public CLI-created RemoteApp session can survive past
#   its original lease after a public remote_desktop.refresh_lease call.
# - It models the daemon/session half of short disconnect resume: the client
#   can validate the same non-terminal session through remote_desktop.show_session
#   and then close it through remote_desktop.end_session.
# - It does not prove long-outage reconnect, browser WebRTC rebind, crash
#   recovery, cross-device network paths, or successful interactive input.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
PROVENANCE_HELPER="$SELF_DIR/remoteapp-evidence-provenance.py"
BUNDLED_SENTINEL_FIXTURE="$REPO_ROOT/tools/scripts/host-remoteapp-sentinel-fixture.sh"
source "$SELF_DIR/remoteapp-lifecycle-harness-lib.sh"

MODE=run
TARGET_KIND=window
OUT_DIR=""
SENTINEL_FIXTURE=0
SENTINEL_FIXTURE_CMD="${EASYNET_REMOTEAPP_SENTINEL_FIXTURE_CMD:-}"
INITIAL_LEASE_TTL_MS="${EASYNET_REMOTEAPP_RESUME_E2E_INITIAL_LEASE_TTL_MS:-2000}"
REFRESH_LEASE_TTL_MS="${EASYNET_REMOTEAPP_RESUME_E2E_REFRESH_LEASE_TTL_MS:-6000}"
WAIT_BEFORE_REFRESH_MS="${EASYNET_REMOTEAPP_RESUME_E2E_WAIT_BEFORE_REFRESH_MS:-250}"
WAIT_AFTER_ORIGINAL_LEASE_MS="${EASYNET_REMOTEAPP_RESUME_E2E_WAIT_AFTER_ORIGINAL_LEASE_MS:-2500}"

usage() {
  cat <<'USAGE'
Usage:
  host-remoteapp-session-resume-e2e.sh --run --sentinel-fixture
  host-remoteapp-session-resume-e2e.sh --self-test

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
  --initial-lease-ttl-ms MS
                        Initial short lease. Default: 2000.
  --refresh-lease-ttl-ms MS
                        Lease requested by remote_desktop.refresh_lease.
                        Default: 6000.
  --wait-before-refresh-ms MS
                        Wait after create before refresh. Default: 250.
  --wait-after-original-lease-ms MS
                        Wait after create before resume validation show_session.
                        Must be greater than initial lease and less than the
                        refreshed expiry window. Default: 2500.
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
        *) echo "invalid resume E2E target kind: $2" >&2; exit 64 ;;
      esac
      shift 2
      ;;
    --sentinel-fixture) SENTINEL_FIXTURE=1; shift ;;
    --sentinel-fixture-cmd)
      SENTINEL_FIXTURE=1
      SENTINEL_FIXTURE_CMD="${2:?missing value for --sentinel-fixture-cmd}"
      shift 2
      ;;
    --initial-lease-ttl-ms) INITIAL_LEASE_TTL_MS="${2:?missing value for --initial-lease-ttl-ms}"; shift 2 ;;
    --refresh-lease-ttl-ms) REFRESH_LEASE_TTL_MS="${2:?missing value for --refresh-lease-ttl-ms}"; shift 2 ;;
    --wait-before-refresh-ms) WAIT_BEFORE_REFRESH_MS="${2:?missing value for --wait-before-refresh-ms}"; shift 2 ;;
    --wait-after-original-lease-ms) WAIT_AFTER_ORIGINAL_LEASE_MS="${2:?missing value for --wait-after-original-lease-ms}"; shift 2 ;;
    --out-dir) OUT_DIR="${2:?missing value for --out-dir}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

TIMESTAMP="$(date -u +%Y%m%d-%H%M%S)"
if [[ -z "$OUT_DIR" ]]; then
  OUT_DIR="$REPO_ROOT/target/e2e/host-remoteapp-session-resume/$TIMESTAMP-$TARGET_KIND-$$"
fi
mkdir -p "$OUT_DIR"

EVIDENCE_JSON="$OUT_DIR/evidence.json"
REPORT_JSON="$OUT_DIR/report.json"
REPORT_MD="$OUT_DIR/report.md"
LIVE_INVENTORY_JSON="$OUT_DIR/live-inventory.json"
SELECTED_RESOURCE_JSON="$OUT_DIR/selected-resource.json"
SENTINEL_MANIFEST_JSON="$OUT_DIR/sentinel-manifest.json"
CREATE_SESSION_JSON="$OUT_DIR/create-session.json"
REFRESH_LEASE_JSON="$OUT_DIR/refresh-lease.json"
SHOW_AFTER_ORIGINAL_LEASE_JSON="$OUT_DIR/show-after-original-lease.json"
END_AFTER_RESUME_JSON="$OUT_DIR/end-after-resume.json"
ABILITY_CATALOG_JSON="$OUT_DIR/ability-catalog.json"
SESSION_ID="rd-session-resume-e2e-$$"

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
  python3 "$PROVENANCE_HELPER" verify --mode "$MODE" --evidence "$EVIDENCE_JSON"
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
refresh = evidence.get("refresh_lease")
show = evidence.get("show_after_original_lease")
ended = evidence.get("end_after_resume")
resource_ura = selected.get("resource_ura") if isinstance(selected, dict) else None

require(evidence.get("status") == "passed", "evidence.status must be passed")
require(evidence.get("proof_mode") == "lease_refresh_resume",
        "proof_mode must be lease_refresh_resume")
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
require(isinstance(evidence.get("initial_lease_ttl_ms"), int)
        and evidence["initial_lease_ttl_ms"] >= 1,
        "initial_lease_ttl_ms must be positive")
require(isinstance(evidence.get("refresh_lease_ttl_ms"), int)
        and evidence["refresh_lease_ttl_ms"] > evidence.get("initial_lease_ttl_ms", 0),
        "refresh_lease_ttl_ms must extend beyond the initial lease")
require(isinstance(evidence.get("wait_after_original_lease_ms"), int)
        and evidence["wait_after_original_lease_ms"] > evidence.get("initial_lease_ttl_ms", 0),
        "resume validation must wait past the original lease")

require(isinstance(create, dict), "create_session evidence must be recorded")
require(get("create_session.ability") == "remote_desktop.create_session",
        "create_session ability must be remote_desktop.create_session")
require(get("create_session.subject_ura") == resource_ura,
        "create_session subject must equal selected Resource URA")
require(get("create_session.exit_code") == 0, "create_session must succeed")
args = get("create_session.args")
require(isinstance(args, dict), "create_session args must be recorded")
if isinstance(args, dict):
    require(args.get("lease_ttl_ms") == evidence.get("initial_lease_ttl_ms"),
            "create_session args must preserve the initial short lease")
    require(not contains_subject_arg(args),
            "create_session args must not carry subject identity")

require(isinstance(refresh, dict), "refresh_lease evidence must be recorded")
require(get("refresh_lease.ability") == "remote_desktop.refresh_lease",
        "refresh_lease ability must be remote_desktop.refresh_lease")
require(get("refresh_lease.subject_ura") == resource_ura,
        "refresh_lease subject must equal selected Resource URA")
require(get("refresh_lease.exit_code") == 0, "refresh_lease must succeed")
require(get("refresh_lease.args.lease_ttl_ms") == evidence.get("refresh_lease_ttl_ms"),
        "refresh_lease args must carry refreshed lease TTL")
require(get("refresh_lease.session.session_id") == get("create_session.session.session_id"),
        "refresh_lease must preserve the original session id")
require(get("refresh_lease.session.subject_ura") == resource_ura,
        "refresh_lease session subject must remain selected Resource URA")
require(get("refresh_lease.session.state") != "closed",
        "refresh_lease must not close the session")
require(get("refresh_lease.session.terminal_receipt") is None,
        "refresh_lease session must remain non-terminal")
require(isinstance(get("create_session.session.lease_expires_at_ms"), int)
        and isinstance(get("refresh_lease.session.lease_expires_at_ms"), int)
        and get("refresh_lease.session.lease_expires_at_ms")
        > get("create_session.session.lease_expires_at_ms"),
        "refresh_lease must extend lease_expires_at_ms")

require(isinstance(show, dict), "show_after_original_lease evidence must be recorded")
require(get("show_after_original_lease.ability") == "remote_desktop.show_session",
        "show_after_original_lease ability must be remote_desktop.show_session")
require(get("show_after_original_lease.subject_ura") == resource_ura,
        "show_after_original_lease subject must equal selected Resource URA")
require(get("show_after_original_lease.exit_code") == 0,
        "show_after_original_lease must succeed")
require(get("show_after_original_lease.waited_past_original_lease") is True,
        "show_after_original_lease must wait past the original lease")
require(get("show_after_original_lease.session.session_id")
        == get("create_session.session.session_id"),
        "show_after_original_lease must preserve the original session id")
require(get("show_after_original_lease.session.state") != "closed",
        "show_after_original_lease must prove the refreshed session survived")
require(get("show_after_original_lease.session.terminal_receipt") is None,
        "show_after_original_lease session must remain non-terminal")
require(isinstance(get("show_after_original_lease.session.lease_expires_at_ms"), int)
        and get("show_after_original_lease.session.lease_expires_at_ms")
        == get("refresh_lease.session.lease_expires_at_ms"),
        "show_after_original_lease must observe the refreshed lease")

require(isinstance(ended, dict), "end_after_resume evidence must be recorded")
require(get("end_after_resume.ability") == "remote_desktop.end_session",
        "end_after_resume ability must be remote_desktop.end_session")
require(get("end_after_resume.subject_ura") == resource_ura,
        "end_after_resume subject must equal selected Resource URA")
require(get("end_after_resume.exit_code") == 0, "end_after_resume must succeed")
require(get("end_after_resume.session.state") == "closed",
        "end_after_resume session must be closed")
require(get("end_after_resume.session.end_reason") == "resume_e2e_cleanup",
        "end_after_resume must use resume_e2e_cleanup terminal reason")
require(get("end_after_resume.session.terminal_receipt.reason_code") == "resume_e2e_cleanup",
        "end_after_resume terminal_receipt.reason_code must be resume_e2e_cleanup")

report = {
    "script": "tools/scripts/host-remoteapp-session-resume-e2e.sh",
    "status": "failed" if errors else "passed",
    "errors": errors,
    "evidence_json": evidence_path,
    "target_kind": evidence.get("target_kind"),
    "selected_resource_ura": resource_ura,
    "session_id": get("create_session.session.session_id"),
    "proof_mode": evidence.get("proof_mode"),
    "waited_past_original_lease": get("show_after_original_lease.waited_past_original_lease"),
    "lifecycle_summary": {
        "kind": "session_resume",
        "proof_mode": evidence.get("proof_mode"),
        "lease_extended": (
            isinstance(get("create_session.session.lease_expires_at_ms"), int)
            and isinstance(get("refresh_lease.session.lease_expires_at_ms"), int)
            and get("refresh_lease.session.lease_expires_at_ms")
            > get("create_session.session.lease_expires_at_ms")
        ),
        "waited_past_original_lease": (
            get("show_after_original_lease.waited_past_original_lease") is True
        ),
        "survived_original_lease": get("show_after_original_lease.session.state") != "closed",
        "same_session_after_refresh": (
            get("refresh_lease.session.session_id")
            == get("create_session.session.session_id")
        ),
        "non_terminal_after_refresh": get("refresh_lease.session.terminal_receipt") is None,
        "non_terminal_after_original_lease": (
            get("show_after_original_lease.session.terminal_receipt") is None
        ),
        "cleanup_terminal_reason": get("end_after_resume.session.terminal_receipt.reason_code"),
        "cleanup_terminal_receipt_visible": get("end_after_resume.session.terminal_receipt.terminal") is True,
        "cleanup_terminal_receipt_session_bound": (
            get("end_after_resume.session.terminal_receipt.session_id")
            == get("create_session.session.session_id")
        ),
        "selected_from_live_refresh": evidence.get("selected_from_live_refresh") is True,
    },
}
pathlib.Path(report_path).write_text(
    json.dumps(report, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
with open(md_path, "w", encoding="utf-8") as f:
    f.write("# RemoteApp session resume E2E report\n\n")
    f.write(f"- Status: `{report['status']}`\n")
    f.write(f"- Target kind: `{report['target_kind']}`\n")
    f.write(f"- Selected Resource URA: `{report['selected_resource_ura']}`\n")
    f.write(f"- Session id: `{report['session_id']}`\n")
    f.write(f"- Proof mode: `{report['proof_mode']}`\n")
    f.write(f"- Waited past original lease: `{report['waited_past_original_lease']}`\n")
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
  python3 "$PROVENANCE_HELPER" project-report --mode "$MODE" \
    --evidence "$EVIDENCE_JSON" --report "$REPORT_JSON"
}

if [[ "$MODE" == "self-test" ]]; then
  python3 - "$EVIDENCE_JSON" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
subject = "easynet:///r/localhost/resource/device.dev/streams/window.resume"
created_session = {
    "session_id": "rd-session-resume-e2e-self-test",
    "subject_ura": subject,
    "state": "negotiating",
    "lease_expires_at_ms": 1_000,
    "terminal_receipt": None,
}
refreshed_session = {
    **created_session,
    "lease_expires_at_ms": 4_000,
}
ended_session = {
    **refreshed_session,
    "state": "closed",
    "end_reason": "resume_e2e_cleanup",
    "terminal_receipt": {
        "receipt_type": "remoteapp.session.terminal.v1",
        "session_id": "rd-session-resume-e2e-self-test",
        "reason_code": "resume_e2e_cleanup",
        "terminal": True,
    },
}
evidence = {
    "status": "passed",
    "evidence_origin": "contract_self_test",
    "proof_mode": "lease_refresh_resume",
    "target_kind": "window",
    "initial_lease_ttl_ms": 2000,
    "refresh_lease_ttl_ms": 6000,
    "wait_after_original_lease_ms": 2500,
    "selected_from_live_refresh": True,
    "selected_resource": {
        "resource_ura": subject,
        "type": "window",
        "display_name": "EasyNet selected resume sentinel fixture",
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
            "session_id": "rd-session-resume-e2e-self-test",
            "mode": "view_only",
            "transport_preferences": ["webrtc"],
            "lease_ttl_ms": 2000,
        },
        "session": created_session,
    },
    "refresh_lease": {
        "ability": "remote_desktop.refresh_lease",
        "subject_ura": subject,
        "exit_code": 0,
        "args": {
            "session_id": "rd-session-resume-e2e-self-test",
            "session_token": "redacted",
            "lease_ttl_ms": 6000,
        },
        "session": refreshed_session,
    },
    "show_after_original_lease": {
        "ability": "remote_desktop.show_session",
        "subject_ura": subject,
        "exit_code": 0,
        "waited_past_original_lease": True,
        "session": refreshed_session,
    },
    "end_after_resume": {
        "ability": "remote_desktop.end_session",
        "subject_ura": subject,
        "exit_code": 0,
        "session": ended_session,
    },
}
path.write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
  validate_evidence
  echo "host-remoteapp-session-resume-e2e self-test ok"
  exit 0
fi

need_cmd python3
for pair in \
  "INITIAL_LEASE_TTL_MS:$INITIAL_LEASE_TTL_MS" \
  "REFRESH_LEASE_TTL_MS:$REFRESH_LEASE_TTL_MS" \
  "WAIT_BEFORE_REFRESH_MS:$WAIT_BEFORE_REFRESH_MS" \
  "WAIT_AFTER_ORIGINAL_LEASE_MS:$WAIT_AFTER_ORIGINAL_LEASE_MS"; do
  name="${pair%%:*}"
  value="${pair#*:}"
  case "$value" in
    ''|*[!0-9]*) die "$name must be a positive integer" ;;
  esac
  (( value >= 1 )) || die "$name must be >= 1"
done
(( REFRESH_LEASE_TTL_MS > INITIAL_LEASE_TTL_MS )) || die "--refresh-lease-ttl-ms must be greater than --initial-lease-ttl-ms"
(( WAIT_BEFORE_REFRESH_MS < INITIAL_LEASE_TTL_MS )) || die "--wait-before-refresh-ms must be less than --initial-lease-ttl-ms"
(( WAIT_AFTER_ORIGINAL_LEASE_MS > INITIAL_LEASE_TTL_MS )) || die "--wait-after-original-lease-ms must be greater than --initial-lease-ttl-ms"
(( WAIT_AFTER_ORIGINAL_LEASE_MS < REFRESH_LEASE_TTL_MS )) || die "--wait-after-original-lease-ms must be less than --refresh-lease-ttl-ms"

if [[ "$TARGET_KIND" != "display" ]]; then
  [[ "$SENTINEL_FIXTURE" == "1" ]] || die "--sentinel-fixture is required for live window/application resume E2E"
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
python3 "$SELF_DIR/remoteapp-select-live-target.py" \
  --inventory "$LIVE_INVENTORY_JSON" \
  --output "$SELECTED_RESOURCE_JSON" \
  --kind "$TARGET_KIND" \
  --pid "${EASYNET_REMOTEAPP_TARGET_PID}" \
  --hint "${EASYNET_REMOTEAPP_TARGET_HINT}"

SELECTED_RESOURCE_URA="$(python3 - "$SELECTED_RESOURCE_JSON" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as f:
    print(json.load(f)["resource_ura"])
PY
)"

run_easynet ability list --format json >"$ABILITY_CATALOG_JSON"
REFRESH_LEASE_ABILITY_URA="$(remoteapp_resolve_rpc_ability_ura "$ABILITY_CATALOG_JSON" remote_desktop.refresh_lease)"
END_SESSION_ABILITY_URA="$(remoteapp_resolve_rpc_ability_ura "$ABILITY_CATALOG_JSON" remote_desktop.end_session)"

run_easynet ability create-remote-desktop-session \
  --subject "$SELECTED_RESOURCE_URA" \
  --session-id "$SESSION_ID" \
  --mode view_only \
  --transport webrtc \
  --lease-ttl-ms "$INITIAL_LEASE_TTL_MS" \
  --format json >"$CREATE_SESSION_JSON"

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

python3 - "$WAIT_BEFORE_REFRESH_MS" <<'PY'
import sys
import time
time.sleep(int(sys.argv[1]) / 1000)
PY

REFRESH_ARGS="$(python3 - "$SESSION_ID" "$SESSION_TOKEN" "$REFRESH_LEASE_TTL_MS" <<'PY'
import json
import sys
print(json.dumps({
    "session_id": sys.argv[1],
    "session_token": sys.argv[2],
    "lease_ttl_ms": int(sys.argv[3]),
}, separators=(",", ":")))
PY
)"
REFRESH_NONCE_HEX="$(python3 - <<'PY'
import secrets
print(secrets.token_hex(16))
PY
)"
REFRESH_RAW_JSON="$OUT_DIR/refresh-lease-raw.txt"
run_easynet ability invoke "$REFRESH_LEASE_ABILITY_URA" \
  --subject "$SELECTED_RESOURCE_URA" \
  --nonce-hex "$REFRESH_NONCE_HEX" \
  --causal-context-json "$SESSION_CAUSAL_CONTEXT_JSON" \
  --args "$REFRESH_ARGS" >"$REFRESH_RAW_JSON"
json_first_value_to_file "$REFRESH_RAW_JSON" "$REFRESH_LEASE_JSON"

python3 - "$WAIT_AFTER_ORIGINAL_LEASE_MS" "$WAIT_BEFORE_REFRESH_MS" <<'PY'
import sys
import time
remaining = max(0, int(sys.argv[1]) - int(sys.argv[2]))
time.sleep(remaining / 1000)
PY

run_easynet ability show-remote-desktop-session \
  --session-json "$CREATE_SESSION_JSON" \
  --format json >"$SHOW_AFTER_ORIGINAL_LEASE_JSON"

END_ARGS="$(python3 - "$SESSION_ID" "$SESSION_TOKEN" <<'PY'
import json
import sys
print(json.dumps({
    "session_id": sys.argv[1],
    "session_token": sys.argv[2],
    "reason": "resume_e2e_cleanup",
}, separators=(",", ":")))
PY
)"
END_NONCE_HEX="$(python3 - <<'PY'
import secrets
print(secrets.token_hex(16))
PY
)"
END_RAW_JSON="$OUT_DIR/end-after-resume-raw.txt"
run_easynet ability invoke "$END_SESSION_ABILITY_URA" \
  --subject "$SELECTED_RESOURCE_URA" \
  --nonce-hex "$END_NONCE_HEX" \
  --causal-context-json "$SESSION_CAUSAL_CONTEXT_JSON" \
  --args "$END_ARGS" >"$END_RAW_JSON"
json_first_value_to_file "$END_RAW_JSON" "$END_AFTER_RESUME_JSON"

python3 - "$EVIDENCE_JSON" "$TARGET_KIND" "$INITIAL_LEASE_TTL_MS" "$REFRESH_LEASE_TTL_MS" "$WAIT_AFTER_ORIGINAL_LEASE_MS" "$SENTINEL_MANIFEST_JSON" "$LIVE_INVENTORY_JSON" "$SELECTED_RESOURCE_JSON" "$CREATE_SESSION_JSON" "$REFRESH_LEASE_JSON" "$SHOW_AFTER_ORIGINAL_LEASE_JSON" "$END_AFTER_RESUME_JSON" "$REFRESH_ARGS" <<'PY'
import json
import pathlib
import sys

(
    evidence_path,
    target_kind,
    initial_lease_ttl_ms,
    refresh_lease_ttl_ms,
    wait_after_original_lease_ms,
    fixture_manifest_path,
    live_inventory_path,
    selected_resource_path,
    create_session_path,
    refresh_lease_path,
    show_after_original_lease_path,
    end_after_resume_path,
    refresh_args_json,
) = sys.argv[1:14]

def load(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)

fixture = load(fixture_manifest_path)
live_inventory = load(live_inventory_path)
selected = load(selected_resource_path)
create_response = load(create_session_path)
refresh_response = load(refresh_lease_path)
show_response = load(show_after_original_lease_path)
end_response = load(end_after_resume_path)
create_session = create_response.get("session")
invocation = create_response.get("invocation")
if not isinstance(create_session, dict):
    raise SystemExit("create-remote-desktop-session response missing session object")
if not isinstance(invocation, dict):
    raise SystemExit("create-remote-desktop-session response missing verified invocation metadata")
if not isinstance(refresh_response, dict):
    raise SystemExit("refresh_lease response missing session object")
if not isinstance(show_response, dict):
    raise SystemExit("show-remote-desktop-session response missing session object")
if not isinstance(end_response, dict):
    raise SystemExit("end_session response missing session object")

refresh_args = json.loads(refresh_args_json)
refresh_args["session_token"] = "redacted"
evidence = {
    "status": "passed",
    "evidence_origin": "live_runner",
    "proof_mode": "lease_refresh_resume",
    "target_kind": target_kind,
    "initial_lease_ttl_ms": int(initial_lease_ttl_ms),
    "refresh_lease_ttl_ms": int(refresh_lease_ttl_ms),
    "wait_after_original_lease_ms": int(wait_after_original_lease_ms),
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
    "refresh_lease": {
        "ability": "remote_desktop.refresh_lease",
        "subject_ura": selected.get("resource_ura"),
        "exit_code": 0,
        "args": refresh_args,
        "session": refresh_response,
    },
    "show_after_original_lease": {
        "ability": "remote_desktop.show_session",
        "subject_ura": selected.get("resource_ura"),
        "exit_code": 0,
        "waited_past_original_lease": True,
        "session": show_response,
    },
    "end_after_resume": {
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
echo "host-remoteapp-session-resume-e2e ok: $REPORT_MD"
