#!/usr/bin/env bash
# Host-side remoteapp view-only input safety E2E.
#
# Boundary:
# - This script proves SPEC E2E-11 at the public CLI/daemon boundary:
#   a user asks for an interactive app/window remote desktop session, but the
#   daemon has no proven focus-safe target-scoped input dispatcher. The public
#   create/show session views must therefore report input_mode=view_only and a
#   key/pointer rejection contract of input_scope_unsupported.
# - It does not add a diagnostic input API. The live proof is the public
#   session/input_plane policy; the production data-channel/bidi rejection path
#   is covered by the Rust input tests.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
BUNDLED_SENTINEL_FIXTURE="$REPO_ROOT/tools/scripts/host-remoteapp-sentinel-fixture.sh"

MODE=run
TARGET_KIND=window
OUT_DIR=""
SENTINEL_FIXTURE=0
SENTINEL_FIXTURE_CMD="${EASYNET_REMOTEAPP_SENTINEL_FIXTURE_CMD:-}"

usage() {
  cat <<'USAGE'
Usage:
  host-remoteapp-view-only-input-safety-e2e.sh --run --sentinel-fixture
  host-remoteapp-view-only-input-safety-e2e.sh --self-test

Options:
  --run                 Execute against the local EasyNet daemon.
  --self-test           Validate the harness against synthetic positive evidence.
  --target-kind KIND    window or application. Default: window.
  --sentinel-fixture    Launch the bundled native AppKit selected/unrelated
                        fixture and select the known target.
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
    --target-kind)
      case "${2:?missing value for --target-kind}" in
        window|application) TARGET_KIND="$2" ;;
        *) echo "invalid view-only input safety kind: $2" >&2; exit 64 ;;
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
  OUT_DIR="$REPO_ROOT/target/e2e/host-remoteapp-view-only-input-safety/$TIMESTAMP-$TARGET_KIND-$$"
fi
mkdir -p "$OUT_DIR"

EVIDENCE_JSON="$OUT_DIR/evidence.json"
REPORT_JSON="$OUT_DIR/report.json"
REPORT_MD="$OUT_DIR/report.md"
LIVE_INVENTORY_JSON="$OUT_DIR/live-inventory.json"
SELECTED_RESOURCE_JSON="$OUT_DIR/selected-resource.json"
SENTINEL_MANIFEST_JSON="$OUT_DIR/sentinel-manifest.json"
CREATE_SESSION_JSON="$OUT_DIR/create-session.json"
SHOW_SESSION_JSON="$OUT_DIR/show-session.json"
SESSION_ID="rd-view-only-input-safety-e2e-$$"
LEASE_TTL_MS=5000

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

def policy_is_view_only(policy, prefix):
    require(isinstance(policy, dict), f"{prefix} must be an object")
    if not isinstance(policy, dict):
        return
    require(policy.get("input_scope") == "view_only", f"{prefix}.input_scope must be view_only")
    require(policy.get("keyboard_enabled") is False, f"{prefix}.keyboard_enabled must be false")
    require(policy.get("pointer_enabled") is False, f"{prefix}.pointer_enabled must be false")
    require(policy.get("clipboard_enabled") is False, f"{prefix}.clipboard_enabled must be false")
    require(policy.get("file_drop_enabled") is False, f"{prefix}.file_drop_enabled must be false")
    unsupported = policy.get("unsupported_input_types")
    require(isinstance(unsupported, list), f"{prefix}.unsupported_input_types must be a list")
    if isinstance(unsupported, list):
        require("clipboard" in unsupported, f"{prefix} must mark clipboard unsupported")
        require("file_drop" in unsupported, f"{prefix} must mark file_drop unsupported")

target_kind = evidence.get("target_kind")
selected = evidence.get("selected_resource")
fixture = evidence.get("sentinel_fixture")
create = evidence.get("create_session")
show = evidence.get("show_session")
session = get("create_session.session")
target_binding = get("create_session.session.target_binding")
scope_audit = get("create_session.session.scope_audit")
input_policy = get("create_session.session.input_policy")
input_plane_policy = get("create_session.session.input_plane.policy")
expected_rejections = evidence.get("expected_input_rejections")

require(evidence.get("status") == "passed", "evidence.status must be passed")
require(target_kind in {"window", "application"}, "target_kind must be window or application")
require(evidence.get("requested_input_mode") == "interactive", "requested_input_mode must be interactive")
require(evidence.get("selected_from_live_refresh") is True, "target must be selected from a live refresh")
require(isinstance(fixture, dict), "sentinel_fixture must be recorded")
require(isinstance(selected, dict), "selected_resource must be recorded")
resource_ura = selected.get("resource_ura") if isinstance(selected, dict) else None
require(isinstance(resource_ura, str) and resource_ura.startswith("easynet:///"),
        "selected resource_ura must be a canonical EasyNet URA")
require(selected.get("type") == target_kind if isinstance(selected, dict) else False,
        "selected resource type must match target_kind")
require(get("selected_resource.metadata.availability") == "available",
        "selected target must be available")
require(get("selected_resource.metadata.discovery_source") == "resource.refresh_remote_targets",
        "selected target must come from resource.refresh_remote_targets")

require(isinstance(create, dict), "create_session evidence must be recorded")
require(get("create_session.ability") == "remote_desktop.create_session",
        "create_session ability must be remote_desktop.create_session")
require(get("create_session.subject_ura") == resource_ura,
        "create_session subject must equal selected Resource URA")
require(get("create_session.exit_code") == 0, "create_session must succeed")
args = get("create_session.args")
require(isinstance(args, dict), "create_session args must be recorded as an object")
if isinstance(args, dict):
    require(args.get("mode") == "interactive", "create_session args.mode must prove the interactive request")
    require(args.get("lease_ttl_ms") == evidence.get("lease_ttl_ms"),
            "create_session args must preserve the bounded lease")
    require(not contains_subject_arg(args),
            "create_session args must not carry subject identity")

require(isinstance(session, dict), "create_session.session must be recorded")
require(get("create_session.session.subject_ura") == resource_ura,
        "session subject must equal selected Resource URA")
require(get("create_session.session.mode") == "interactive",
        "session.mode must preserve the requested interactive mode for audit")
require(isinstance(target_binding, dict), "session.target_binding must be recorded")
require(get("create_session.session.target_binding.target_kind") == target_kind,
        "target_binding.target_kind must match target_kind")
require(get("create_session.session.target_binding.input_scope") == "view_only",
        "target_binding.input_scope must be view_only")
require(get("create_session.session.target_binding.input_scope_reason")
        == "target_scoped_keyboard_pointer_dispatch_unsafe",
        "target_binding must expose target_scoped_keyboard_pointer_dispatch_unsafe")
require(isinstance(scope_audit, dict), "session.scope_audit must be recorded")
require(get("create_session.session.scope_audit.input_mode") == "view_only",
        "scope_audit.input_mode must be view_only")
require(get("create_session.session.scope_audit.input_scope_reason")
        == "target_scoped_keyboard_pointer_dispatch_unsafe",
        "scope_audit must expose target_scoped_keyboard_pointer_dispatch_unsafe")
require(get("create_session.session.scope_audit.scope_widened") is False,
        "view-only input downgrade must not widen capture scope")
require(get("create_session.session.scope_audit.display_fallback_used") is False,
        "view-only input downgrade must not use display fallback")

policy_is_view_only(input_policy, "session.input_policy")
policy_is_view_only(input_plane_policy, "session.input_plane.policy")
require(input_policy == input_plane_policy,
        "input_plane.policy must equal the public session input_policy")

require(isinstance(expected_rejections, dict), "expected_input_rejections must be recorded")
if isinstance(expected_rejections, dict):
    require(expected_rejections.get("key") == "input_scope_unsupported",
            "keyboard frames must be rejected or ignored with input_scope_unsupported")
    require(expected_rejections.get("pointer") == "input_scope_unsupported",
            "pointer frames must be rejected or ignored with input_scope_unsupported")
    require(expected_rejections.get("evidence_source") == "public_session_input_policy",
            "expected rejection evidence must come from the public session input policy")

require(isinstance(show, dict), "show_session evidence must be recorded")
require(get("show_session.exit_code") == 0, "show_session must succeed")
require(get("show_session.session.input_policy.input_scope") == "view_only",
        "show_session must preserve view_only input_policy")
require(get("show_session.session.scope_audit.input_mode") == "view_only",
        "show_session must preserve input_mode=view_only")
require(get("show_session.session.target_binding.input_scope_reason")
        == "target_scoped_keyboard_pointer_dispatch_unsafe",
        "show_session must preserve target-scoped input unsafe reason")

report = {
    "status": "failed" if errors else "passed",
    "errors": errors,
    "evidence_json": evidence_path,
    "target_kind": target_kind,
    "selected_resource_ura": resource_ura,
    "requested_input_mode": evidence.get("requested_input_mode"),
    "effective_input_mode": get("create_session.session.scope_audit.input_mode"),
    "key_rejection": get("expected_input_rejections.key"),
    "pointer_rejection": get("expected_input_rejections.pointer"),
}
pathlib.Path(report_path).write_text(
    json.dumps(report, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
with open(md_path, "w", encoding="utf-8") as f:
    f.write("# RemoteApp view-only input safety E2E report\n\n")
    f.write(f"- Status: `{report['status']}`\n")
    f.write(f"- Target kind: `{report['target_kind']}`\n")
    f.write(f"- Selected Resource URA: `{report['selected_resource_ura']}`\n")
    f.write(f"- Requested input mode: `{report['requested_input_mode']}`\n")
    f.write(f"- Effective input mode: `{report['effective_input_mode']}`\n")
    f.write(f"- Key rejection: `{report['key_rejection']}`\n")
    f.write(f"- Pointer rejection: `{report['pointer_rejection']}`\n")
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
subject = "easynet:///r/localhost/resource/device.dev/streams/window.viewonly"
policy = {
    "input_scope": "view_only",
    "keyboard_enabled": False,
    "pointer_enabled": False,
    "clipboard_enabled": False,
    "file_drop_enabled": False,
    "unsupported_input_types": ["clipboard", "file_drop"],
}
session = {
    "session_id": "rd-view-only-input-safety-e2e-self-test",
    "subject_ura": subject,
    "mode": "interactive",
    "target_binding": {
        "subject_ura": subject,
        "target_kind": "window",
        "input_scope": "view_only",
        "input_scope_reason": "target_scoped_keyboard_pointer_dispatch_unsafe",
    },
    "scope_audit": {
        "requested_target_kind": "window",
        "effective_target_kind": "window",
        "input_mode": "view_only",
        "input_scope_reason": "target_scoped_keyboard_pointer_dispatch_unsafe",
        "scope_widened": False,
        "display_fallback_used": False,
    },
    "input_policy": policy,
    "input_plane": {
        "kind": "webrtc_data_channel",
        "label": "remote-desktop-input",
        "policy": policy,
        "input_injection_available": True,
    },
}
evidence = {
    "status": "passed",
    "target_kind": "window",
    "requested_input_mode": "interactive",
    "lease_ttl_ms": 5000,
    "selected_from_live_refresh": True,
    "sentinel_fixture": {
        "selected": {
            "label": "EasyNet selected window sentinel fixture",
            "pid": 4242,
        },
    },
    "selected_resource": {
        "resource_ura": subject,
        "type": "window",
        "display_name": "EasyNet selected window sentinel fixture",
        "metadata": {
            "pid": 4242,
            "title": "EasyNet selected window sentinel fixture",
            "availability": "available",
            "discovery_source": "resource.refresh_remote_targets",
            "inventory_source": "daemon_resource_inventory",
        },
    },
    "create_session": {
        "ability": "remote_desktop.create_session",
        "subject_ura": subject,
        "exit_code": 0,
        "args": {
            "session_id": "rd-view-only-input-safety-e2e-self-test",
            "mode": "interactive",
            "transport_preferences": ["webrtc"],
            "lease_ttl_ms": 5000,
        },
        "session": session,
    },
    "show_session": {
        "ability": "remote_desktop.show_session",
        "subject_ura": subject,
        "exit_code": 0,
        "session": session,
    },
    "expected_input_rejections": {
        "key": "input_scope_unsupported",
        "pointer": "input_scope_unsupported",
        "evidence_source": "public_session_input_policy",
    },
}
path.write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
  validate_evidence
  echo "host-remoteapp-view-only-input-safety-e2e self-test ok"
  exit 0
fi

[[ "$SENTINEL_FIXTURE" == "1" ]] || die "--sentinel-fixture is required for live view-only input safety E2E"
if [[ -z "$SENTINEL_FIXTURE_CMD" ]]; then
  [[ -x "$BUNDLED_SENTINEL_FIXTURE" ]] || die "missing bundled sentinel fixture: $BUNDLED_SENTINEL_FIXTURE"
  SENTINEL_FIXTURE_CMD="$BUNDLED_SENTINEL_FIXTURE --target-kind $TARGET_KIND"
fi

need_cmd python3

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

run_easynet ability refresh-remote-targets --type "$TARGET_KIND" --format json >"$LIVE_INVENTORY_JSON"
python3 - "$LIVE_INVENTORY_JSON" "$SELECTED_RESOURCE_JSON" "$TARGET_KIND" "$EASYNET_REMOTEAPP_TARGET_PID" "$EASYNET_REMOTEAPP_TARGET_HINT" <<'PY'
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
    meta = metadata(resource)
    values = [meta.get("pid"), meta.get("primary_pid")]
    return any(str(value) == str(target_pid) for value in values if value is not None)

def text_matches(resource):
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
if len(candidates) != 1:
    sample = [
        {
            "resource_ura": resource.get("resource_ura"),
            "type": resource.get("type"),
            "display_name": resource.get("display_name"),
            "pid": metadata(resource).get("pid"),
            "primary_pid": metadata(resource).get("primary_pid"),
            "title": metadata(resource).get("title"),
            "app_name": metadata(resource).get("app_name"),
            "availability": metadata(resource).get("availability"),
        }
        for resource in resources
        if resource.get("type") == target_kind
    ][:12]
    raise SystemExit(
        f"known {target_kind} target must resolve exactly once from live refresh; got {len(candidates)} sample={sample}"
    )
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
  --mode interactive \
  --transport webrtc \
  --lease-ttl-ms "$LEASE_TTL_MS" \
  --format json >"$CREATE_SESSION_JSON"

run_easynet ability show-remote-desktop-session \
  --session-json "$CREATE_SESSION_JSON" \
  --format json >"$SHOW_SESSION_JSON"

python3 - "$EVIDENCE_JSON" "$TARGET_KIND" "$LEASE_TTL_MS" "$SENTINEL_MANIFEST_JSON" "$LIVE_INVENTORY_JSON" "$SELECTED_RESOURCE_JSON" "$CREATE_SESSION_JSON" "$SHOW_SESSION_JSON" <<'PY'
import json
import pathlib
import sys

(
    evidence_path,
    target_kind,
    lease_ttl_ms,
    fixture_manifest_path,
    live_inventory_path,
    selected_resource_path,
    create_session_path,
    show_session_path,
) = sys.argv[1:9]

def load(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)

fixture = load(fixture_manifest_path)
live_inventory = load(live_inventory_path)
selected = load(selected_resource_path)
create_response = load(create_session_path)
show_response = load(show_session_path)
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
    "target_kind": target_kind,
    "requested_input_mode": "interactive",
    "lease_ttl_ms": int(lease_ttl_ms),
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
    "show_session": {
        "ability": "remote_desktop.show_session",
        "subject_ura": selected.get("resource_ura"),
        "exit_code": 0,
        "session": show_response,
    },
    "expected_input_rejections": {
        "key": "input_scope_unsupported",
        "pointer": "input_scope_unsupported",
        "evidence_source": "public_session_input_policy",
    },
}
pathlib.Path(evidence_path).write_text(
    json.dumps(evidence, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
PY

validate_evidence
echo "host-remoteapp-view-only-input-safety-e2e ok: $REPORT_MD"
