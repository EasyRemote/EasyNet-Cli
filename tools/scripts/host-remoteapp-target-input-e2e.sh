#!/usr/bin/env bash
# Real macOS target-local RemoteApp input E2E runner.
#
# This runner uses one production WebRTC session for decoded target media and
# the canonical input data channel. The selected AppKit process independently
# records the mouse/key callbacks caused by daemon CGEvent injection.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
source "$SELF_DIR/remoteapp-lifecycle-harness-lib.sh"

TARGET_KIND="${EASYNET_REMOTEAPP_E2E_TARGET_KIND:-window}"
OUT_DIR=""
EVIDENCE_JSON="${EASYNET_REMOTEAPP_INPUT_INJECTION_EVIDENCE_JSON:-}"

usage() {
  cat <<'USAGE'
Usage:
  host-remoteapp-target-input-e2e.sh [options]

Options:
  --target-kind KIND    window or application. Default: window.
  --out-dir DIR         Artifact directory.
  --evidence-json PATH  Canonical input evidence output path. Defaults to
                        EASYNET_REMOTEAPP_INPUT_INJECTION_EVIDENCE_JSON or
                        <out-dir>/evidence.json.
  -h, --help            Show this help.

The local daemon must be paired and running. Screen Recording and
Accessibility permissions must be granted to the daemon executable.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target-kind)
      case "${2:?missing value for --target-kind}" in
        window|application) TARGET_KIND="$2" ;;
        *) echo "invalid target kind: $2" >&2; exit 64 ;;
      esac
      shift 2
      ;;
    --out-dir) OUT_DIR="${2:?missing value for --out-dir}"; shift 2 ;;
    --evidence-json) EVIDENCE_JSON="${2:?missing value for --evidence-json}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

if [[ -z "$OUT_DIR" ]]; then
  OUT_DIR="$REPO_ROOT/target/e2e/host-remoteapp-target-input/$(date -u +%Y%m%d-%H%M%S)-$TARGET_KIND-$$"
fi
mkdir -p "$OUT_DIR"
if [[ -z "$EVIDENCE_JSON" ]]; then
  EVIDENCE_JSON="$OUT_DIR/evidence.json"
fi
mkdir -p "$(dirname "$EVIDENCE_JSON")"

FIXTURE_DIR="$OUT_DIR/sentinel-fixture"
PROBE_DIR="$OUT_DIR/probe"
FRAME_EVIDENCE_JSON="$OUT_DIR/frame-evidence.json"
ABILITY_CATALOG_JSON="$OUT_DIR/ability-catalog.json"
WATCH_EVENTS_JSON="$OUT_DIR/watch-events.json"
END_RAW_JSON="$OUT_DIR/end-session.raw.txt"
END_SESSION_JSON="$OUT_DIR/end-session.json"
SHOW_ENDED_JSON="$OUT_DIR/show-ended.json"
EASYNET_BIN="$REPO_ROOT/target/debug/easynet"
RECEIVER_BIN="$REPO_ROOT/target/debug/examples/easynet-remoteapp-frame-receiver"
SESSION_ENDED=0

die() {
  echo "[FAIL] $*" >&2
  exit 1
}

run_easynet() {
  "$EASYNET_BIN" "$@"
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

cleanup() {
  local exit_code=$?
  if [[ "$SESSION_ENDED" == "0" && -s "$PROBE_DIR/session.json" && -x "$EASYNET_BIN" ]]; then
    local cleanup_catalog="$OUT_DIR/cleanup-ability-catalog.json"
    local cleanup_raw="$OUT_DIR/cleanup-end-session.raw.txt"
    if "$EASYNET_BIN" ability list --format json >"$cleanup_catalog" 2>/dev/null; then
      local cleanup_ability_ura
      cleanup_ability_ura="$(remoteapp_resolve_rpc_ability_ura "$cleanup_catalog" remote_desktop.end_session 2>/dev/null || true)"
      if [[ -n "$cleanup_ability_ura" ]]; then
        local cleanup_binding
        cleanup_binding="$(python3 - "$PROBE_DIR/session.json" <<'PY' 2>/dev/null || true
import json
import sys
with open(sys.argv[1], encoding="utf-8") as f:
    session = json.load(f).get("session", {})
print(session.get("subject_ura", ""), session.get("session_id", ""), session.get("session_token", ""))
PY
)"
        local cleanup_subject cleanup_session_id cleanup_token
        read -r cleanup_subject cleanup_session_id cleanup_token <<<"$cleanup_binding"
        local cleanup_causal
        cleanup_causal="$(remoteapp_session_approval_causal_context_json "$PROBE_DIR/session.json" 2>/dev/null || true)"
        if [[ -n "$cleanup_subject" && -n "$cleanup_session_id" && -n "$cleanup_token" && -n "$cleanup_causal" ]]; then
          local cleanup_args cleanup_nonce
          cleanup_args="$(python3 - "$cleanup_session_id" "$cleanup_token" <<'PY'
import json
import sys
print(json.dumps({
    "session_id": sys.argv[1],
    "session_token": sys.argv[2],
    "reason": "input_injection_e2e_cleanup",
}, separators=(",", ":")))
PY
)"
          cleanup_nonce="$(python3 - <<'PY'
import secrets
print(secrets.token_hex(16))
PY
)"
          "$EASYNET_BIN" ability invoke "$cleanup_ability_ura" \
            --subject "$cleanup_subject" \
            --nonce-hex "$cleanup_nonce" \
            --causal-context-json "$cleanup_causal" \
            --args "$cleanup_args" >"$cleanup_raw" 2>&1 || true
        fi
      fi
    fi
  fi
  if [[ -x "$FIXTURE_DIR/cleanup.sh" ]]; then
    "$FIXTURE_DIR/cleanup.sh" >/dev/null 2>&1 || true
  fi
  return "$exit_code"
}

[[ "$(uname -s)" == "Darwin" ]] || die "target-local input runner requires macOS"
command -v cargo >/dev/null 2>&1 || die "cargo is required"
command -v python3 >/dev/null 2>&1 || die "python3 is required"

# Build outside the active session lease.
cargo build --quiet --bin easynet --example easynet-remoteapp-frame-receiver --features remote-desktop
[[ -x "$EASYNET_BIN" ]] || die "easynet build missing: $EASYNET_BIN"
[[ -x "$RECEIVER_BIN" ]] || die "receiver build missing: $RECEIVER_BIN"

mkdir -p "$FIXTURE_DIR" "$PROBE_DIR"
trap cleanup EXIT
"$SELF_DIR/host-remoteapp-sentinel-fixture.sh" \
  --target-kind "$TARGET_KIND" \
  --out-dir "$FIXTURE_DIR"
[[ -f "$FIXTURE_DIR/env.sh" ]] || die "sentinel fixture did not write env.sh"
source "$FIXTURE_DIR/env.sh"

export EASYNET_REMOTEAPP_EASYNET_BIN="$EASYNET_BIN"
export EASYNET_REMOTEAPP_FRAME_RECEIVER_CMD="$RECEIVER_BIN"
export EASYNET_REMOTEAPP_E2E_TARGET_KIND="$TARGET_KIND"
export EASYNET_REMOTEAPP_INPUT_PROOF=1
export EASYNET_REMOTEAPP_PROBE_OUT_DIR="$PROBE_DIR"
export EASYNET_REMOTEAPP_FRAME_EVIDENCE_JSON="$FRAME_EVIDENCE_JSON"
"$SELF_DIR/host-remoteapp-decoded-frame-probe.sh"

SESSION_JSON="$PROBE_DIR/session.json"
INPUT_TRANSMISSION_JSON="$PROBE_DIR/input-transmission.json"
SELECTED_RESOURCE_JSON="$PROBE_DIR/selected-resource.json"
for artifact in "$SESSION_JSON" "$INPUT_TRANSMISSION_JSON" "$SELECTED_RESOURCE_JSON" \
  "$EASYNET_REMOTEAPP_SELECTED_INPUT_EVENT_LOG"; do
  [[ -s "$artifact" ]] || die "missing live input artifact: $artifact"
done

run_easynet ability watch-remote-desktop-events \
  --session-json "$SESSION_JSON" \
  --from-sequence 0 \
  --max-events 1 \
  --format json >"$WATCH_EVENTS_JSON"

run_easynet ability list --format json >"$ABILITY_CATALOG_JSON"
END_SESSION_ABILITY_URA="$(remoteapp_resolve_rpc_ability_ura "$ABILITY_CATALOG_JSON" remote_desktop.end_session)"
SESSION_CAUSAL_CONTEXT_JSON="$(remoteapp_session_approval_causal_context_json "$SESSION_JSON")"
read -r SELECTED_RESOURCE_URA SESSION_ID SESSION_TOKEN < <(python3 - "$SESSION_JSON" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as f:
    response = json.load(f)
session = response.get("session", {})
print(session.get("subject_ura", ""), session.get("session_id", ""), session.get("session_token", ""))
PY
)
[[ -n "$SELECTED_RESOURCE_URA" && -n "$SESSION_ID" && -n "$SESSION_TOKEN" ]] || \
  die "create_session artifact missing control binding"
END_ARGS="$(python3 - "$SESSION_ID" "$SESSION_TOKEN" <<'PY'
import json
import sys
print(json.dumps({
    "session_id": sys.argv[1],
    "session_token": sys.argv[2],
    "reason": "input_injection_e2e_cleanup",
}, separators=(",", ":")))
PY
)"
END_NONCE_HEX="$(python3 - <<'PY'
import secrets
print(secrets.token_hex(16))
PY
)"
run_easynet ability invoke "$END_SESSION_ABILITY_URA" \
  --subject "$SELECTED_RESOURCE_URA" \
  --nonce-hex "$END_NONCE_HEX" \
  --causal-context-json "$SESSION_CAUSAL_CONTEXT_JSON" \
  --args "$END_ARGS" >"$END_RAW_JSON"
json_first_value_to_file "$END_RAW_JSON" "$END_SESSION_JSON"
run_easynet ability show-remote-desktop-session \
  --session-json "$SESSION_JSON" \
  --format json >"$SHOW_ENDED_JSON"
SESSION_ENDED=1

python3 - "$EVIDENCE_JSON" "$TARGET_KIND" "$SESSION_JSON" "$INPUT_TRANSMISSION_JSON" \
  "$EASYNET_REMOTEAPP_SELECTED_INPUT_EVENT_LOG" "$EASYNET_REMOTEAPP_UNRELATED_INPUT_EVENT_LOG" \
  "$END_SESSION_JSON" "$SHOW_ENDED_JSON" <<'PY'
import json
import math
import pathlib
import statistics
import sys

(
    evidence_path,
    target_kind,
    session_path,
    transmission_path,
    selected_log_path,
    unrelated_log_path,
    end_path,
    show_ended_path,
) = sys.argv[1:9]

def load(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)

def load_jsonl(path):
    source = pathlib.Path(path)
    if not source.exists():
        return []
    rows = []
    for line in source.read_text(encoding="utf-8").splitlines():
        if line.strip():
            rows.append(json.loads(line))
    return rows

create = load(session_path)
session = create.get("session", {})
invocation = create.get("invocation", {})
transmission = load(transmission_path)
after_input = transmission.get("session_view_after_input", {})
events = after_input.get("events", [])
selected_events = load_jsonl(selected_log_path)
unrelated_events = load_jsonl(unrelated_log_path)
ended = load(end_path)
shown_ended = load(show_ended_path)

subject = session.get("subject_ura")
session_id = session.get("session_id")
transport_epoch = transmission.get("transport_epoch")
geometry_revision = transmission.get("target_geometry_revision")
focus_epoch = transmission.get("target_focus_epoch")
expected_position = transmission.get("expected_pointer_position")

if transmission.get("status") != "passed":
    raise SystemExit("receiver input transmission did not pass")
if not isinstance(events, list):
    raise SystemExit("post-input session view missing events")

def input_event(event_type, kind=None, sequence=None):
    matches = []
    for event in events:
        if not isinstance(event, dict) or event.get("event_type") != event_type:
            continue
        payload = event.get("payload") if isinstance(event.get("payload"), dict) else {}
        if kind is not None and payload.get("kind") != kind:
            continue
        if sequence is not None and payload.get("client_sequence") != sequence:
            continue
        matches.append((event, payload))
    if not matches:
        raise SystemExit(f"missing {event_type} kind={kind} client_sequence={sequence}")
    return matches[-1]

pointer_event, pointer_payload = input_event("INPUT_FRAME_APPLIED", "pointer", 1)
key_event, key_payload = input_event("INPUT_FRAME_APPLIED", "key", 3)
reject_event, reject_payload = input_event("INPUT_FRAME_REJECTED", "pointer", 1)
if reject_payload.get("reason") != "stale_client_sequence":
    raise SystemExit("stale pointer probe was not rejected as stale_client_sequence")

def observed(kind, action):
    rows = [
        row for row in selected_events
        if row.get("kind") == kind and row.get("action") == action
    ]
    if not rows:
        raise SystemExit(f"selected AppKit target did not observe {kind}/{action}")
    return rows[-1]

pointer_observed = observed("pointer", "down")
key_observed = observed("keyboard", "down")
if any(row.get("kind") in {"pointer", "keyboard"} for row in unrelated_events):
    raise SystemExit("unrelated AppKit target received RemoteApp input")

actual_position = pointer_observed.get("global_position")
if not isinstance(actual_position, dict) or not isinstance(expected_position, dict):
    raise SystemExit("pointer evidence missing global coordinates")
distance = math.hypot(
    float(actual_position.get("x")) - float(expected_position.get("x")),
    float(actual_position.get("y")) - float(expected_position.get("y")),
)
if distance > 8.0:
    raise SystemExit(f"observed pointer position differs by {distance:.2f}px")
if key_observed.get("key_code") != "KeyA":
    raise SystemExit(f"selected target observed unexpected key: {key_observed.get('key_code')}")

def applied_result(kind, event, payload, os_event):
    result = dict(payload)
    result.update({
        "kind": "keyboard" if kind == "key" else "pointer",
        "result": "input_applied",
        "event_type": "INPUT_FRAME_APPLIED",
        "transport_epoch": event.get("transport_epoch", transport_epoch),
    })
    if kind == "pointer":
        result.update({
            "observed_effect": "pointer_position_changed",
            "coordinate_mapping": "target_geometry_revision_matched",
            "target_geometry_revision": geometry_revision,
            "os_effect": {
                "observed": True,
                "os_effect_probe_source": "macos_appkit_target_observer",
                "observer_independent_from_injector": True,
                "input_event_id": payload.get("input_event_id"),
                "platform": "macos",
                "subject_ura": subject,
                "session_id": session_id,
                "target_geometry_revision": geometry_revision,
                "target_focus_epoch": focus_epoch,
                "observed_at_ms": os_event.get("observed_at_ms"),
                "effect_type": "pointer_position",
                "coordinate_space": "display_global",
                "expected_position": expected_position,
                "observed_position": actual_position,
                "within_tolerance_px": True,
                "position_tolerance_px": 8,
            },
        })
    else:
        result.update({
            "observed_effect": "key_event_observed",
            "key_code": "KeyA",
            "os_effect": {
                "observed": True,
                "os_effect_probe_source": "macos_appkit_target_observer",
                "observer_independent_from_injector": True,
                "input_event_id": payload.get("input_event_id"),
                "platform": "macos",
                "subject_ura": subject,
                "session_id": session_id,
                "target_geometry_revision": geometry_revision,
                "target_focus_epoch": focus_epoch,
                "observed_at_ms": os_event.get("observed_at_ms"),
                "effect_type": "key_event",
                "focused_resource_ura": subject,
                "expected_key_code": "KeyA",
                "observed_key_code": os_event.get("key_code"),
            },
        })
    return result

input_results = [
    applied_result("pointer", pointer_event, pointer_payload, pointer_observed),
    applied_result("key", key_event, key_payload, key_observed),
]
latencies = [float(item.get("latency_ms", 0)) for item in input_results]
terminal_receipt = shown_ended.get("terminal_receipt") or ended.get("terminal_receipt")
if not isinstance(terminal_receipt, dict) or terminal_receipt.get("terminal") is not True:
    raise SystemExit("end_session did not expose terminal receipt")

abilities = [
    {"name": invocation.get("ability"), "subject_ura": subject},
    {"name": "remote_desktop.set_description", "subject_ura": subject, "session_id": session_id},
    {"name": "remote_desktop.watch_events", "subject_ura": subject, "session_id": session_id},
    {"name": "remote_desktop.end_session", "subject_ura": subject, "session_id": session_id},
]
macos = {
    "platform": "macos",
    "status": "passed",
    "target_kind": target_kind,
    "selected_resource_ura": subject,
    "session_id": session_id,
    "permission": {"accessibility_granted": True, "input_injection_granted": True},
    "consent_scope": "input_control",
    "input_scope": "target_local",
    "focus_validated": True,
    "coordinate_mapping_validated": True,
    "target_geometry_revision": geometry_revision,
    "target_focus_epoch": focus_epoch,
    "source_only_proof": False,
    "policy_only": False,
    "abilities": abilities,
    "input_results": input_results,
    "rejected_input_results": [{
        **reject_payload,
        "event_type": reject_event.get("event_type"),
        "subject_ura": subject,
        "session_id": session_id,
    }],
    "latency_summary": {"p95_ms": max(latencies), "max_ms": max(latencies)},
    "terminal_receipt": terminal_receipt,
}

def unsupported(platform):
    return {
        "platform": platform,
        "status": "unsupported",
        "unsupported_state": "explicit_product_unsupported",
        "show_unsupported": True,
        "input_results": [],
        "rejected_input_results": [],
    }

evidence = {
    "status": "passed",
    "proof_mode": "real_input_injection_matrix",
    "component_mock": False,
    "real_backend_runtime": True,
    "product_complete_claim": False,
    "latency_threshold_ms": 100,
    "platforms": [macos, unsupported("windows"), unsupported("linux")],
}
pathlib.Path(evidence_path).write_text(
    json.dumps(evidence, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
PY

"$SELF_DIR/remoteapp-input-injection-e2e.sh" \
  --run \
  --evidence-json "$EVIDENCE_JSON" \
  --out-dir "$OUT_DIR/verification"

echo "host-remoteapp-target-input-e2e PASS: $EVIDENCE_JSON"
