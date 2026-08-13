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

TARGET_HINT="${EASYNET_REMOTEAPP_TARGET_HINT:-}"
TARGET_RESOURCE_URA="${EASYNET_REMOTEAPP_TARGET_RESOURCE_URA:-}"
DEFAULT_FRAME_RECEIVER_CMD="cargo run --quiet --example easynet-remoteapp-frame-receiver --features remote-desktop --"
if [[ -x "$REPO_ROOT/target/debug/examples/easynet-remoteapp-frame-receiver" ]]; then
  DEFAULT_FRAME_RECEIVER_CMD="$REPO_ROOT/target/debug/examples/easynet-remoteapp-frame-receiver"
fi
FRAME_RECEIVER_CMD="${EASYNET_REMOTEAPP_FRAME_RECEIVER_CMD:-$DEFAULT_FRAME_RECEIVER_CMD}"

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

need_cmd python3

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

run_easynet ability refresh-remote-targets \
  --type "$TARGET_KIND" \
  --format json >"$LIVE_INVENTORY_JSON"

python3 - "$LIVE_INVENTORY_JSON" "$SELECTED_RESOURCE_JSON" "$TARGET_KIND" "$TARGET_HINT" "$TARGET_RESOURCE_URA" <<'PY'
import json
import sys

inventory_path, selected_path, target_kind, hint, target_resource_ura = sys.argv[1:6]
with open(inventory_path, encoding="utf-8") as f:
    inventory = json.load(f)

resources = inventory.get("resources")
if not isinstance(resources, list):
    raise SystemExit("resource.refresh_remote_targets response missing resources array")

def availability(resource):
    metadata = resource.get("metadata") if isinstance(resource.get("metadata"), dict) else {}
    return metadata.get("availability", "available")

def search_blob(resource):
    metadata = resource.get("metadata") if isinstance(resource.get("metadata"), dict) else {}
    fields = [
        resource.get("resource_ura"),
        resource.get("display_name"),
        metadata.get("app_name"),
        metadata.get("title"),
        metadata.get("bundle_id"),
        metadata.get("app_identity"),
    ]
    return "\n".join(str(value).lower() for value in fields if value)

candidates = [
    resource for resource in resources
    if resource.get("type") == target_kind and availability(resource) == "available"
]

if target_resource_ura:
    candidates = [
        resource for resource in candidates
        if resource.get("resource_ura") == target_resource_ura
    ]
    if not candidates:
        raise SystemExit(f"selected target Resource URA is not in live {target_kind} inventory: {target_resource_ura}")
elif hint:
    needle = hint.lower()
    candidates = [resource for resource in candidates if needle in search_blob(resource)]
elif len(candidates) != 1:
    sample = [
        {
            "resource_ura": resource.get("resource_ura"),
            "display_name": resource.get("display_name"),
            "type": resource.get("type"),
        }
        for resource in candidates[:10]
    ]
    raise SystemExit(
        "remoteapp host probe refuses ambiguous target selection; set "
        "EASYNET_REMOTEAPP_TARGET_HINT or EASYNET_REMOTEAPP_TARGET_RESOURCE_URA. "
        f"candidate_count={len(candidates)} sample={json.dumps(sample, sort_keys=True)}"
    )

if len(candidates) != 1:
    raise SystemExit(f"remoteapp target selection must resolve exactly one {target_kind}; got {len(candidates)}")

resource_ura = candidates[0].get("resource_ura")
if not isinstance(resource_ura, str) or not resource_ura.startswith("easynet:///"):
    raise SystemExit("selected target must expose a canonical EasyNet Resource URA")

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
  --mode view_only \
  --transport webrtc \
  --format json >"$SESSION_JSON"

rm -f "$FRAME_ANALYSIS_JSON"
export EASYNET_REMOTEAPP_LIVE_INVENTORY_JSON="$LIVE_INVENTORY_JSON"
export EASYNET_REMOTEAPP_SELECTED_RESOURCE_JSON="$SELECTED_RESOURCE_JSON"
export EASYNET_REMOTEAPP_SELECTED_RESOURCE_URA="$SELECTED_RESOURCE_URA"
export EASYNET_REMOTEAPP_SESSION_JSON="$SESSION_JSON"
export EASYNET_REMOTEAPP_FRAME_ANALYSIS_JSON="$FRAME_ANALYSIS_JSON"
export EASYNET_REMOTEAPP_E2E_TARGET_KIND="$TARGET_KIND"

bash -lc "$FRAME_RECEIVER_CMD"
[[ -s "$FRAME_ANALYSIS_JSON" ]] || die "frame receiver did not write frame analysis JSON: $FRAME_ANALYSIS_JSON"

python3 - "$LIVE_INVENTORY_JSON" "$SELECTED_RESOURCE_JSON" "$SESSION_JSON" "$FRAME_ANALYSIS_JSON" "$EVIDENCE_JSON" "$TARGET_KIND" <<'PY'
import json
import os
import sys

inventory_path, selected_path, session_path, frame_path, evidence_path, expected_kind = sys.argv[1:7]

def load(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)

inventory = load(inventory_path)
selected = load(selected_path)
session_response = load(session_path)
frame = load(frame_path)

session = session_response.get("session")
invocation = session_response.get("invocation")
if not isinstance(session, dict):
    raise SystemExit("create-remote-desktop-session response missing session object")
if not isinstance(invocation, dict):
    raise SystemExit("create-remote-desktop-session response missing verified invocation metadata")

target_binding = session.get("target_binding")
if not isinstance(target_binding, dict):
    raise SystemExit("session response missing target_binding")

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

decoded_frames = frame.get("decoded_frames")
if not isinstance(decoded_frames, dict):
    raise SystemExit("frame analysis missing decoded_frames object")
artifacts = frame.get("artifacts")
if not isinstance(artifacts, dict):
    raise SystemExit("frame analysis missing artifacts object")

transport = frame.get("transport")
if transport is None:
    transport = {"kind": "webrtc"}
if not isinstance(transport, dict):
    raise SystemExit("frame analysis transport must be an object")

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

selected_label = os.environ.get("EASYNET_REMOTEAPP_SELECTED_SENTINEL_LABEL", "").strip()
unrelated_label = os.environ.get("EASYNET_REMOTEAPP_UNRELATED_SENTINEL_LABEL", "").strip()
if not selected_label:
    raise SystemExit("EASYNET_REMOTEAPP_SELECTED_SENTINEL_LABEL is required")
if not unrelated_label:
    raise SystemExit("EASYNET_REMOTEAPP_UNRELATED_SENTINEL_LABEL is required")
if selected_label == unrelated_label:
    raise SystemExit("selected and unrelated sentinel labels must be distinct")

unrelated_fixture = {
    "label": unrelated_label,
    "placement": os.environ.get(
        "EASYNET_REMOTEAPP_UNRELATED_SENTINEL_PLACEMENT",
        "other_application" if expected_kind == "application" else "other_window",
    ),
    "rgb": parse_rgb_env("EASYNET_REMOTEAPP_UNRELATED_SENTINEL_RGB"),
}
unrelated_resource_ura = os.environ.get("EASYNET_REMOTEAPP_UNRELATED_SENTINEL_RESOURCE_URA", "").strip()
if unrelated_resource_ura:
    unrelated_fixture["resource_ura"] = unrelated_resource_ura

evidence = {
    "status": "passed" if frame.get("status") == "passed" else "failed",
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
        "callee_ura": invocation.get("callee_ura"),
        "request_id": invocation.get("request_id"),
        "receipt": invocation.get("receipt"),
    },
    "target_binding": {
        "target_kind": target_binding.get("target_kind"),
        "capture_scope": target_binding.get("capture_scope"),
        "binding_id": target_binding.get("binding_id"),
        "binding_epoch": target_binding.get("binding_epoch"),
        "target_identity_epoch": target_binding.get("target_identity_epoch"),
        "target_geometry_revision": target_binding.get("target_geometry_revision"),
        "resolved_identity": target_binding.get("resolved_identity"),
        "scope_audit": {
            "scope_widened": scope_audit.get("scope_widened"),
            "display_fallback_used": scope_audit.get("display_fallback_used"),
        },
    },
    "sentinel_fixture": {
        "proof": "dual_target_non_leak",
        "selected": {
            "label": selected_label,
            "resource_ura": selected_resource_ura,
            "rgb": parse_rgb_env("EASYNET_REMOTEAPP_SELECTED_SENTINEL_RGB"),
            "target_kind": expected_kind,
        },
        "unrelated": unrelated_fixture,
    },
    "transport": transport,
    "decoded_frames": decoded_frames,
    "artifacts": artifacts,
}

if expected_kind == "application":
    evidence["target_binding"]["app_window_set"] = target_binding.get("app_window_set")

with open(evidence_path, "w", encoding="utf-8") as f:
    json.dump(evidence, f, indent=2, sort_keys=True)
    f.write("\n")
PY
