#!/usr/bin/env bash
# Host-side remote app/window decoded-frame probe.
#
# Boundary:
# - This script owns the EasyNet control-plane part of the host E2E:
#   live target inventory -> selected Resource URA -> create_session.
# - The bundled frame receiver owns WebRTC/H.264 decode and pixel assertions.
#   Callers may override it with EASYNET_REMOTEAPP_FRAME_RECEIVER_CMD for
#   equivalent host rigs. The receiver must write frame-analysis JSON; this
#   script folds that into the canonical evidence consumed by
#   host-remoteapp-decoded-frame-e2e.sh.
# - The script fails closed when the selected target is ambiguous, when the
#   receiver is missing, or when the receiver omits decoded-frame assertions.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"

TARGET_KIND="${EASYNET_REMOTEAPP_E2E_TARGET_KIND:-window}"
case "$TARGET_KIND" in
  window|application) ;;
  *) echo "[FAIL] invalid EASYNET_REMOTEAPP_E2E_TARGET_KIND: $TARGET_KIND" >&2; exit 64 ;;
esac
LIFECYCLE_SCENARIO="${EASYNET_REMOTEAPP_LIFECYCLE_SCENARIO:-none}"
case "$LIFECYCLE_SCENARIO" in
  none|move-resize|target-loss) ;;
  *) echo "[FAIL] invalid EASYNET_REMOTEAPP_LIFECYCLE_SCENARIO: $LIFECYCLE_SCENARIO" >&2; exit 64 ;;
esac
PRE_MEDIA_RESOURCE_REFRESH="${EASYNET_REMOTEAPP_PRE_MEDIA_RESOURCE_REFRESH:-0}"
case "$PRE_MEDIA_RESOURCE_REFRESH" in
  0|1) ;;
  *) echo "[FAIL] invalid EASYNET_REMOTEAPP_PRE_MEDIA_RESOURCE_REFRESH: $PRE_MEDIA_RESOURCE_REFRESH" >&2; exit 64 ;;
esac
INPUT_PROOF="${EASYNET_REMOTEAPP_INPUT_PROOF:-0}"
case "$INPUT_PROOF" in
  0|1) ;;
  *) echo "[FAIL] invalid EASYNET_REMOTEAPP_INPUT_PROOF: $INPUT_PROOF" >&2; exit 64 ;;
esac

EVIDENCE_JSON="${EASYNET_REMOTEAPP_FRAME_EVIDENCE_JSON:-}"
[[ -n "$EVIDENCE_JSON" ]] || {
  echo "[FAIL] EASYNET_REMOTEAPP_FRAME_EVIDENCE_JSON is required" >&2
  exit 64
}

OUT_DIR="${EASYNET_REMOTEAPP_PROBE_OUT_DIR:-$(dirname "$EVIDENCE_JSON")}"
mkdir -p "$OUT_DIR"

LIVE_INVENTORY_JSON="$OUT_DIR/live-inventory.json"
SELECTED_RESOURCE_JSON="$OUT_DIR/selected-resource.json"
SESSION_JSON="$OUT_DIR/session.json"
FRAME_ANALYSIS_JSON="$OUT_DIR/frame-analysis.json"
INPUT_TRANSMISSION_JSON="$OUT_DIR/input-transmission.json"
LIFECYCLE_EVENTS_JSON="$OUT_DIR/lifecycle-events.json"
LIFECYCLE_SESSION_JSON="$OUT_DIR/lifecycle-session.json"
PRE_MEDIA_REFRESH_JSON="$OUT_DIR/pre-media-refresh.json"

TARGET_HINT="${EASYNET_REMOTEAPP_TARGET_HINT:-}"
TARGET_RESOURCE_URA="${EASYNET_REMOTEAPP_TARGET_RESOURCE_URA:-}"
TARGET_PID="${EASYNET_REMOTEAPP_TARGET_PID:-}"
BUNDLED_FRAME_RECEIVER_BIN="$REPO_ROOT/target/debug/examples/easynet-remoteapp-frame-receiver"
FRAME_RECEIVER_CMD="${EASYNET_REMOTEAPP_FRAME_RECEIVER_CMD:-$BUNDLED_FRAME_RECEIVER_BIN}"
CAMPAIGN_BINDING_JSON="${EASYNET_REMOTEAPP_CAMPAIGN_BINDING_JSON:-}"
CAMPAIGN_PROOF_BINDING_JSON="${EASYNET_REMOTEAPP_CAMPAIGN_PROOF_BINDING_JSON:-}"
CAMPAIGN_RECEIPT_PROOF_SET_JSON="${EASYNET_REMOTEAPP_CAMPAIGN_RECEIPT_PROOF_SET_JSON:-}"
CAMPAIGN_CREATE_NONCE_HEX="${EASYNET_REMOTEAPP_CAMPAIGN_CREATE_NONCE_HEX:-}"

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

prepare_bundled_frame_receiver() {
  if [[ -n "${EASYNET_REMOTEAPP_FRAME_RECEIVER_CMD:-}" ]]; then
    return 0
  fi
  need_cmd cargo
  cargo build --quiet --example easynet-remoteapp-frame-receiver --features remote-desktop
  [[ -x "$BUNDLED_FRAME_RECEIVER_BIN" ]] || die \
    "bundled frame receiver build did not produce executable: $BUNDLED_FRAME_RECEIVER_BIN"
  FRAME_RECEIVER_CMD="$BUNDLED_FRAME_RECEIVER_BIN"
}

need_cmd python3

campaign_values=(
  "$CAMPAIGN_BINDING_JSON"
  "$CAMPAIGN_PROOF_BINDING_JSON"
  "$CAMPAIGN_RECEIPT_PROOF_SET_JSON"
  "$CAMPAIGN_CREATE_NONCE_HEX"
)
campaign_value_count=0
for campaign_value in "${campaign_values[@]}"; do
  [[ -n "$campaign_value" ]] && campaign_value_count=$((campaign_value_count + 1))
done
if [[ "$campaign_value_count" != "0" && "$campaign_value_count" != "4" ]]; then
  die "campaign mode requires binding, proof binding, proof-set, and create nonce together"
fi

[[ -n "${EASYNET_REMOTEAPP_SELECTED_SENTINEL_RGB:-}" ]] || die \
  "EASYNET_REMOTEAPP_SELECTED_SENTINEL_RGB is required; bundled receiver needs selected target RGB sentinel as r,g,b"
[[ -n "${EASYNET_REMOTEAPP_UNRELATED_SENTINEL_RGB:-}" ]] || die \
  "EASYNET_REMOTEAPP_UNRELATED_SENTINEL_RGB is required; bundled receiver needs unrelated display RGB sentinel as r,g,b"
[[ -n "${EASYNET_REMOTEAPP_SELECTED_SENTINEL_LABEL:-}" ]] || die \
  "EASYNET_REMOTEAPP_SELECTED_SENTINEL_LABEL is required; host rig must label the selected target witness"
[[ -n "${EASYNET_REMOTEAPP_UNRELATED_SENTINEL_LABEL:-}" ]] || die \
  "EASYNET_REMOTEAPP_UNRELATED_SENTINEL_LABEL is required; host rig must label the unrelated non-target witness"

if [[ -z "${EASYNET_REMOTEAPP_EASYNET_BIN:-}" && -x "$REPO_ROOT/target/debug/easynet" ]]; then
  export EASYNET_REMOTEAPP_EASYNET_BIN="$REPO_ROOT/target/debug/easynet"
fi

# The host E2E must not spend an active remote_desktop session lease compiling
# its receiver. Build/readiness belongs before live target selection and
# create_session; after create_session the probe should only negotiate, receive,
# decode, and report evidence.
prepare_bundled_frame_receiver

TARGET_SELECTOR_ARGS=(
  --inventory "$LIVE_INVENTORY_JSON"
  --output "$SELECTED_RESOURCE_JSON"
  --kind "$TARGET_KIND"
)
[[ -n "$TARGET_RESOURCE_URA" ]] && TARGET_SELECTOR_ARGS+=(--resource-ura "$TARGET_RESOURCE_URA")
[[ -n "$TARGET_PID" ]] && TARGET_SELECTOR_ARGS+=(--pid "$TARGET_PID")
[[ -n "$TARGET_HINT" ]] && TARGET_SELECTOR_ARGS+=(--hint "$TARGET_HINT")
TARGET_SELECTION_ERROR="$OUT_DIR/target-selection.stderr.txt"
TARGET_SELECTED=0
for _ in {1..12}; do
  if [[ -x "${EASYNET_REMOTEAPP_SELECTED_CONTROL_SH:-}" ]]; then
    "$EASYNET_REMOTEAPP_SELECTED_CONTROL_SH" focus >/dev/null 2>&1 || true
  fi
  run_easynet ability refresh-remote-targets \
    --type "$TARGET_KIND" \
    --format json >"$LIVE_INVENTORY_JSON"
  if python3 "$SELF_DIR/remoteapp-select-live-target.py" \
      "${TARGET_SELECTOR_ARGS[@]}" 2>"$TARGET_SELECTION_ERROR"; then
    TARGET_SELECTED=1
    break
  fi
  sleep 0.15
done
if [[ "$TARGET_SELECTED" != "1" ]]; then
  die "live target selection did not converge: $(<"$TARGET_SELECTION_ERROR")"
fi

SELECTED_RESOURCE_URA="$(python3 - "$SELECTED_RESOURCE_JSON" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as f:
    print(json.load(f)["resource_ura"])
PY
)"

CREATE_SESSION_ARGS=(
  ability create-remote-desktop-session
  --subject "$SELECTED_RESOURCE_URA"
)
if [[ "$INPUT_PROOF" == "1" ]]; then
  CREATE_SESSION_ARGS+=(--mode interactive --input-control)
else
  CREATE_SESSION_ARGS+=(--mode view_only)
fi
CREATE_SESSION_ARGS+=(--transport webrtc)

if [[ "$campaign_value_count" == "4" ]]; then
  CAMPAIGN_SESSION_ID="$(python3 - "$CAMPAIGN_PROOF_BINDING_JSON" "$SELECTED_RESOURCE_URA" <<'PY'
import json
import sys

binding = json.load(open(sys.argv[1], encoding="utf-8"))
if binding.get("subject_ura") != sys.argv[2]:
    raise SystemExit("campaign proof subject does not match selected Resource")
session_id = binding.get("session_id")
if not isinstance(session_id, str) or not session_id:
    raise SystemExit("campaign proof binding omits session_id")
print(session_id)
PY
)"
  CREATE_SESSION_ARGS+=(--session-id "$CAMPAIGN_SESSION_ID" --nonce-hex "$CAMPAIGN_CREATE_NONCE_HEX")
fi

CREATE_SESSION_ARGS+=(--format json)
run_easynet "${CREATE_SESSION_ARGS[@]}" >"$SESSION_JSON"

if [[ "$campaign_value_count" == "4" ]]; then
  CAMPAIGN_CREATE_META_JSON="$OUT_DIR/campaign-create-invocation-meta.json"
  CAMPAIGN_CREATE_ARGS_JSON="$OUT_DIR/campaign-create-invocation-args.json"
  python3 - "$SESSION_JSON" "$CAMPAIGN_CREATE_META_JSON" "$CAMPAIGN_CREATE_ARGS_JSON" <<'PY'
import json
import pathlib
import sys

response = json.load(open(sys.argv[1], encoding="utf-8"))
meta = response.get("invocation")
if not isinstance(meta, dict):
    raise SystemExit("campaign create_session response omits verified invocation metadata")
args = meta.get("args")
if not isinstance(args, dict):
    raise SystemExit("campaign create_session metadata omits arguments")
pathlib.Path(sys.argv[2]).write_text(json.dumps(meta, indent=2, sort_keys=True) + "\n")
pathlib.Path(sys.argv[3]).write_text(json.dumps(args, indent=2, sort_keys=True) + "\n")
PY
  python3 "$REPO_ROOT/tools/scripts/remoteapp-evidence-provenance.py" append-receipt-proof \
    --proof-set "$CAMPAIGN_RECEIPT_PROOF_SET_JSON" \
    --campaign-binding "$CAMPAIGN_BINDING_JSON" \
    --proof-binding "$CAMPAIGN_PROOF_BINDING_JSON" \
    --arguments-json "$CAMPAIGN_CREATE_ARGS_JSON" \
    --invocation-meta "$CAMPAIGN_CREATE_META_JSON"
fi

if [[ "$PRE_MEDIA_RESOURCE_REFRESH" == "1" ]]; then
  run_easynet ability refresh-remote-targets \
    --type "$TARGET_KIND" \
    --format json >"$PRE_MEDIA_REFRESH_JSON"
fi

rm -f "$FRAME_ANALYSIS_JSON"
export EASYNET_REMOTEAPP_LIVE_INVENTORY_JSON="$LIVE_INVENTORY_JSON"
export EASYNET_REMOTEAPP_SELECTED_RESOURCE_JSON="$SELECTED_RESOURCE_JSON"
export EASYNET_REMOTEAPP_SELECTED_RESOURCE_URA="$SELECTED_RESOURCE_URA"
export EASYNET_REMOTEAPP_SESSION_JSON="$SESSION_JSON"
export EASYNET_REMOTEAPP_FRAME_ANALYSIS_JSON="$FRAME_ANALYSIS_JSON"
export EASYNET_REMOTEAPP_E2E_TARGET_KIND="$TARGET_KIND"
if [[ "$INPUT_PROOF" == "1" ]]; then
  rm -f "$INPUT_TRANSMISSION_JSON"
  export EASYNET_REMOTEAPP_INPUT_TRANSMISSION_JSON="$INPUT_TRANSMISSION_JSON"
fi

bash -lc "$FRAME_RECEIVER_CMD"
[[ -s "$FRAME_ANALYSIS_JSON" ]] || die "frame receiver did not write frame analysis JSON: $FRAME_ANALYSIS_JSON"
if [[ "$INPUT_PROOF" == "1" ]]; then
  [[ -s "$INPUT_TRANSMISSION_JSON" ]] || die \
    "frame receiver did not write input transmission JSON: $INPUT_TRANSMISSION_JSON"
fi

run_lifecycle_scenario() {
  [[ "$LIFECYCLE_SCENARIO" == "none" ]] && return 0
  [[ "$TARGET_KIND" == "window" ]] || die "lifecycle scenario $LIFECYCLE_SCENARIO currently requires a window target"
  [[ -x "${EASYNET_REMOTEAPP_SELECTED_CONTROL_SH:-}" ]] || die \
    "lifecycle scenario requires EASYNET_REMOTEAPP_SELECTED_CONTROL_SH from the sentinel fixture"

  local action
  case "$LIFECYCLE_SCENARIO" in
    move-resize) action="move-resize" ;;
    target-loss) action="close" ;;
    *) die "unsupported lifecycle scenario: $LIFECYCLE_SCENARIO" ;;
  esac

  "$EASYNET_REMOTEAPP_SELECTED_CONTROL_SH" "$action"

  python3 - "$REPO_ROOT" "$SESSION_JSON" "$LIFECYCLE_EVENTS_JSON" "$LIFECYCLE_SESSION_JSON" "$LIFECYCLE_SCENARIO" <<'PY'
import json
import pathlib
import subprocess
import sys
import time

repo_root, session_json, events_json, lifecycle_session_json, scenario = sys.argv[1:6]
easynet = pathlib.Path(repo_root) / "target" / "debug" / "easynet"
cmd_base = [str(easynet) if easynet.exists() else "easynet"]

with open(session_json, encoding="utf-8") as f:
    initial_session = json.load(f)
initial_revision = (
    initial_session.get("target_binding", {}).get("target_geometry_revision")
    if isinstance(initial_session, dict)
    else None
)

def show_session():
    shown = subprocess.run(
        cmd_base + [
            "ability",
            "show-remote-desktop-session",
            "--session-json",
            session_json,
            "--format",
            "json",
        ],
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        timeout=8,
    )
    return json.loads(shown.stdout), shown.stdout

def event_types(session):
    return [
        event.get("event_type")
        for event in session.get("events", [])
        if isinstance(event, dict)
    ]

def scenario_ready(session):
    types = event_types(session)
    if scenario == "move-resize":
        current_revision = session.get("target_tracking", {}).get("target_geometry_revision")
        return (
            "TARGET_MOVED" in types
            and "TARGET_RESIZED" in types
            and isinstance(current_revision, int)
            and isinstance(initial_revision, int)
            and current_revision > initial_revision
        )
    if scenario == "target-loss":
        return (
            "TARGET_LOST" in types
            and "MEDIA_SOURCE_LOST" in types
            and session.get("state") == "suspended"
            and session.get("target_tracking", {}).get("input_enabled") is False
        )
    return True

session = {}
raw = "{}\n"
deadline = time.time() + 12.0
while True:
    session, raw = show_session()
    if scenario_ready(session) or time.time() >= deadline:
        break
    time.sleep(0.35)

pathlib.Path(lifecycle_session_json).write_text(raw, encoding="utf-8")
pathlib.Path(events_json).write_text(
    json.dumps(session.get("events", []), indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
PY
}

run_lifecycle_scenario

python3 - "$LIVE_INVENTORY_JSON" "$SELECTED_RESOURCE_JSON" "$SESSION_JSON" "$FRAME_ANALYSIS_JSON" "$EVIDENCE_JSON" "$TARGET_KIND" "$LIFECYCLE_SCENARIO" "$LIFECYCLE_EVENTS_JSON" "$LIFECYCLE_SESSION_JSON" "$PRE_MEDIA_RESOURCE_REFRESH" "$PRE_MEDIA_REFRESH_JSON" <<'PY'
import json
import os
import sys

(
    inventory_path,
    selected_path,
    session_path,
    frame_path,
    evidence_path,
    expected_kind,
    lifecycle_scenario,
    lifecycle_events_path,
    lifecycle_session_path,
    pre_media_resource_refresh,
    pre_media_refresh_path,
) = sys.argv[1:12]

def load(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)

inventory = load(inventory_path)
selected = load(selected_path)
session_response = load(session_path)
frame = load(frame_path)
lifecycle_events = []
lifecycle_session = None
if lifecycle_scenario != "none":
    lifecycle_events = load(lifecycle_events_path)
    lifecycle_session = load(lifecycle_session_path)
pre_media_refresh = None
if pre_media_resource_refresh == "1":
    pre_media_refresh = load(pre_media_refresh_path)

session = session_response.get("session")
invocation = session_response.get("invocation")
if not isinstance(session, dict):
    raise SystemExit("create-remote-desktop-session response missing session object")
if not isinstance(invocation, dict):
    raise SystemExit("create-remote-desktop-session response missing verified invocation metadata")

initial_target_binding = session.get("target_binding")
if not isinstance(initial_target_binding, dict):
    raise SystemExit("session response missing target_binding")
latest_session = frame.get("session_view")
target_binding = (
    latest_session.get("target_binding")
    if isinstance(latest_session, dict)
    else None
)
if not isinstance(target_binding, dict):
    target_binding = initial_target_binding

scope_audit = session.get("scope_audit")
if not isinstance(scope_audit, dict):
    scope_audit = target_binding.get("scope_audit")
if not isinstance(scope_audit, dict):
    raise SystemExit("session response missing scope_audit")

selected_resource_ura = selected.get("resource_ura")
if invocation.get("subject_ura") != selected_resource_ura:
    raise SystemExit("verified Invocation.subject does not match selected resource_ura")
if invocation.get("ability") != "remote_desktop.create_session":
    raise SystemExit("verified invocation ability is not remote_desktop.create_session")
invocation_args = invocation.get("args")
if not isinstance(invocation_args, dict):
    raise SystemExit("verified remote_desktop.create_session invocation metadata missing args object")

decoded_frames = frame.get("decoded_frames")
if not isinstance(decoded_frames, dict):
    raise SystemExit("frame analysis missing decoded_frames object")
decoded_audio = frame.get("decoded_audio")
expected_audio_frequency = os.environ.get(
    "EASYNET_REMOTEAPP_EXPECTED_AUDIO_FREQUENCY_HZ", ""
).strip()
if expected_audio_frequency and not isinstance(decoded_audio, dict):
    raise SystemExit("frame analysis missing decoded_audio object for audio-required proof")
artifacts = frame.get("artifacts")
if not isinstance(artifacts, dict):
    raise SystemExit("frame analysis missing artifacts object")

transport = frame.get("transport")
if transport is None:
    transport = {"kind": "webrtc"}
if not isinstance(transport, dict):
    raise SystemExit("frame analysis transport must be an object")
production_readiness = frame.get("production_readiness")
if not isinstance(production_readiness, dict):
    raise SystemExit("frame analysis missing production_readiness object from post-negotiation session view")
if production_readiness.get("client_media_ready") is not True:
    raise SystemExit(
        "frame analysis production_readiness.client_media_ready must be true after decoded-frame presentation"
    )

def parse_rgb_env(name):
    raw = os.environ.get(name, "").strip()
    if not raw:
        raise SystemExit(f"{name} is required")
    try:
        values = [int(part.strip()) for part in raw.split(",")]
    except ValueError as exc:
        raise SystemExit(f"{name} must be formatted as r,g,b") from exc
    if len(values) != 3 or any(value < 0 or value > 255 for value in values):
        raise SystemExit(f"{name} must contain exactly three RGB bytes")
    return values

def optional_rgb_env(name):
    if not os.environ.get(name, "").strip():
        return None
    return parse_rgb_env(name)

selected_label = os.environ.get("EASYNET_REMOTEAPP_SELECTED_SENTINEL_LABEL", "").strip()
unrelated_label = os.environ.get("EASYNET_REMOTEAPP_UNRELATED_SENTINEL_LABEL", "").strip()
selected_pid = os.environ.get("EASYNET_REMOTEAPP_SELECTED_SENTINEL_PID", "").strip()
unrelated_pid = os.environ.get("EASYNET_REMOTEAPP_UNRELATED_SENTINEL_PID", "").strip()
if not selected_label:
    raise SystemExit("EASYNET_REMOTEAPP_SELECTED_SENTINEL_LABEL is required")
if not unrelated_label:
    raise SystemExit("EASYNET_REMOTEAPP_UNRELATED_SENTINEL_LABEL is required")
if selected_label == unrelated_label:
    raise SystemExit("selected and unrelated sentinel labels must be distinct")
if selected_pid and (not selected_pid.isdigit() or int(selected_pid) <= 0):
    raise SystemExit("EASYNET_REMOTEAPP_SELECTED_SENTINEL_PID must be a positive integer when set")
if unrelated_pid and (not unrelated_pid.isdigit() or int(unrelated_pid) <= 0):
    raise SystemExit("EASYNET_REMOTEAPP_UNRELATED_SENTINEL_PID must be a positive integer when set")

unrelated_fixture = {
    "label": unrelated_label,
    "placement": os.environ.get(
        "EASYNET_REMOTEAPP_UNRELATED_SENTINEL_PLACEMENT",
        "other_application" if expected_kind == "application" else "other_window",
    ),
    "rgb": parse_rgb_env("EASYNET_REMOTEAPP_UNRELATED_SENTINEL_RGB"),
}
if expected_audio_frequency:
    unrelated_fixture["audio_tone_frequency_hz"] = float(
        os.environ["EASYNET_REMOTEAPP_UNRELATED_AUDIO_FREQUENCY_HZ"]
    )
if unrelated_pid:
    unrelated_fixture["pid"] = int(unrelated_pid)
unrelated_resource_ura = os.environ.get("EASYNET_REMOTEAPP_UNRELATED_SENTINEL_RESOURCE_URA", "").strip()
if unrelated_resource_ura:
    unrelated_fixture["resource_ura"] = unrelated_resource_ura

selected_rgb = parse_rgb_env("EASYNET_REMOTEAPP_SELECTED_SENTINEL_RGB")
selected_surfaces = [{
    "role": "primary",
    "label": selected_label,
    "rgb": selected_rgb,
}]
selected_secondary_rgb = optional_rgb_env(
    "EASYNET_REMOTEAPP_SELECTED_SECONDARY_SENTINEL_RGB"
)
selected_secondary_label = os.environ.get(
    "EASYNET_REMOTEAPP_SELECTED_SECONDARY_SENTINEL_LABEL", ""
).strip()
if selected_secondary_rgb is not None:
    if not selected_secondary_label:
        raise SystemExit(
            "EASYNET_REMOTEAPP_SELECTED_SECONDARY_SENTINEL_LABEL is required when secondary RGB is set"
        )
    selected_surfaces.append({
        "role": "secondary",
        "label": selected_secondary_label,
        "rgb": selected_secondary_rgb,
    })

evidence = {
    "status": "passed" if frame.get("status") == "passed" else "failed",
    "evidence_origin": "live_runner",
    "live_inventory": {
        "ability": "resource.refresh_remote_targets",
        "observed_at_ms": inventory.get("observed_at_ms"),
        "freshness_ttl_ms": inventory.get("freshness_ttl_ms"),
        "returned_resource_count": len(inventory.get("resources", [])),
    },
    "selected_resource_ura": selected_resource_ura,
    "session_id": session.get("session_id"),
    "invocation": {
        "ability": invocation.get("ability"),
        "subject_ura": invocation.get("subject_ura"),
        "args": invocation.get("args"),
        "callee_ura": invocation.get("callee_ura"),
        "request_id": invocation.get("request_id"),
        "receipt": invocation.get("receipt"),
    },
    "target_binding": {
        "subject_ura": target_binding.get("subject_ura"),
        "target_kind": target_binding.get("target_kind"),
        "capture_scope": target_binding.get("capture_scope"),
        "binding_id": target_binding.get("binding_id"),
        "binding_epoch": target_binding.get("binding_epoch"),
        "target_identity_epoch": target_binding.get("target_identity_epoch"),
        "target_geometry_revision": target_binding.get("target_geometry_revision"),
        "media_source_epoch": target_binding.get("media_source_epoch"),
        "consent_epoch": target_binding.get("consent_epoch"),
        "resolved_identity": target_binding.get("resolved_identity"),
        "scope_audit": {
            "scope_widened": scope_audit.get("scope_widened"),
            "display_fallback_used": scope_audit.get("display_fallback_used"),
        },
    },
    "initial_target_binding": {
        "binding_id": initial_target_binding.get("binding_id"),
        "binding_epoch": initial_target_binding.get("binding_epoch"),
        "target_identity_epoch": initial_target_binding.get("target_identity_epoch"),
        "target_geometry_revision": initial_target_binding.get("target_geometry_revision"),
        "media_source_epoch": initial_target_binding.get("media_source_epoch"),
    },
    "sentinel_fixture": {
        "proof": "dual_target_non_leak",
        "selected": {
            "label": selected_label,
            "resource_ura": selected_resource_ura,
            "rgb": selected_rgb,
            "surfaces": selected_surfaces,
            "target_kind": expected_kind,
            "pid": int(selected_pid) if selected_pid else None,
        },
        "unrelated": unrelated_fixture,
    },
    "transport": transport,
    "production_media_ready": frame.get("production_media_ready"),
    "production_readiness": production_readiness,
    "decoded_frames": decoded_frames,
    "decoded_audio": decoded_audio,
    "artifacts": artifacts,
}

if expected_audio_frequency:
    evidence["sentinel_fixture"]["selected"]["audio_tone_frequency_hz"] = float(
        expected_audio_frequency
    )

if expected_kind == "application":
    evidence["target_binding"]["app_window_set"] = target_binding.get("app_window_set")

if pre_media_resource_refresh == "1":
    refreshed_resources = (
        pre_media_refresh.get("resources")
        if isinstance(pre_media_refresh, dict)
        else None
    )
    if not isinstance(refreshed_resources, list):
        raise SystemExit("pre-media resource refresh response missing resources array")
    selected_after_refresh = next(
        (
            resource
            for resource in refreshed_resources
            if isinstance(resource, dict)
            and resource.get("resource_ura") == selected_resource_ura
            and resource.get("type") == expected_kind
        ),
        None,
    )
    selected_refresh_metadata = (
        selected_after_refresh.get("metadata")
        if isinstance(selected_after_refresh, dict)
        and isinstance(selected_after_refresh.get("metadata"), dict)
        else {}
    )
    evidence["pre_media_resource_refresh"] = {
        "ability": "resource.refresh_remote_targets",
        "after_create_session": True,
        "before_media_start": True,
        "selected_resource_ura": selected_resource_ura,
        "returned_resource_count": len(refreshed_resources),
        "selected_resource_still_live": selected_after_refresh is not None
        and selected_refresh_metadata.get("availability", "available") == "available",
        "selected_resource_freshness_source": (
            selected_refresh_metadata.get("freshness", {}).get("source")
            if isinstance(selected_refresh_metadata.get("freshness"), dict)
            else None
        ),
        "session_binding_id": target_binding.get("binding_id"),
        "session_binding_epoch": target_binding.get("binding_epoch"),
        "target_identity_epoch": target_binding.get("target_identity_epoch"),
        "target_geometry_revision": target_binding.get("target_geometry_revision"),
        "media_source_epoch": target_binding.get("media_source_epoch"),
    }

if lifecycle_scenario != "none":
    lifecycle = {
        "scenario": lifecycle_scenario,
        "events": lifecycle_events,
        "session": lifecycle_session,
    }
    if lifecycle_scenario == "move-resize":
        lifecycle["required_events"] = ["TARGET_MOVED", "TARGET_RESIZED"]
    elif lifecycle_scenario == "target-loss":
        lifecycle["required_events"] = ["TARGET_LOST", "MEDIA_SOURCE_LOST"]
    evidence["lifecycle"] = lifecycle

with open(evidence_path, "w", encoding="utf-8") as f:
    json.dump(evidence, f, indent=2, sort_keys=True)
    f.write("\n")
PY
