#!/usr/bin/env bash
# Host-gated decoded-frame E2E for remote app/window sessions.
#
# This script intentionally does NOT run by default. It validates the SPEC's
# strongest host-only evidence requirement: after a live picker selects a
# window/application resource URA and remote_desktop.create_session starts a
# WebRTC session, decoded media frames must contain the selected target and
# must not contain unrelated full-display sentinel content.
#
# The script is a harness boundary, not a static source check. With --run it
# runs the bundled EasyNet host probe by default. The bundled probe performs
# live inventory/session invocation and defaults to the bundled frame receiver
# for WebRTC decode + pixel assertions. Callers may still provide --probe-cmd
# to use an equivalent external probe.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
BUNDLED_PROBE="$SELF_DIR/host-remoteapp-decoded-frame-probe.sh"
BUNDLED_SENTINEL_FIXTURE="$SELF_DIR/host-remoteapp-sentinel-fixture.sh"
TIMESTAMP="$(date +%Y%m%d-%H%M%S)"
OUT_DIR="${EASYNET_REMOTEAPP_E2E_OUT_DIR:-}"
RUN="${EASYNET_HOST_REMOTEAPP_DECODED_FRAME_E2E:-0}"
SELF_TEST=0
PROBE_CMD="${EASYNET_REMOTEAPP_FRAME_PROBE_CMD:-}"
PROBE_CMD_USES_BUNDLED=0
TARGET_KIND="${EASYNET_REMOTEAPP_E2E_TARGET_KIND:-window}"
SENTINEL_FIXTURE="${EASYNET_REMOTEAPP_SENTINEL_FIXTURE:-0}"
SENTINEL_FIXTURE_CMD="${EASYNET_REMOTEAPP_SENTINEL_FIXTURE_CMD:-}"
DEFAULT_SENTINEL_TOLERANCE=64

usage() {
  cat <<'USAGE'
Usage:
  tools/scripts/host-remoteapp-decoded-frame-e2e.sh [options]

Options:
  --run                 Run the host decoded-frame E2E harness.
  --probe-cmd CMD       Command that drives the real GUI/WebRTC probe and writes
                        EASYNET_REMOTEAPP_FRAME_EVIDENCE_JSON. If omitted,
                        uses tools/scripts/host-remoteapp-decoded-frame-probe.sh.
  --target-kind KIND    Target kind: window or application. Default: window.
  --out-dir DIR         Report directory.
  --sentinel-fixture    Launch the bundled host sentinel fixture before the
                        probe and source its env.sh. This creates visible
                        selected/unrelated native windows; it does not fake
                        inventory, session creation, media, or pixel evidence.
  --sentinel-fixture-cmd CMD
                        Equivalent fixture command override. The command
                        receives EASYNET_REMOTEAPP_SENTINEL_FIXTURE_DIR and
                        EASYNET_REMOTEAPP_E2E_TARGET_KIND and must write
                        env.sh plus an optional cleanup.sh into that directory.
  --self-test           Validate harness structure with a synthetic probe.
  -h, --help            Show this help.

Environment:
  EASYNET_HOST_REMOTEAPP_DECODED_FRAME_E2E=1
                        Equivalent to --run.
  EASYNET_REMOTEAPP_FRAME_PROBE_CMD
                        Same as --probe-cmd.
  EASYNET_REMOTEAPP_FRAME_RECEIVER_CMD
                        Optional override for the bundled receiver. The command
                        must perform WebRTC receive/decode/pixel assertions and
                        write EASYNET_REMOTEAPP_FRAME_ANALYSIS_JSON.
  EASYNET_REMOTEAPP_SENTINEL_FIXTURE=1
                        Equivalent to --sentinel-fixture.
  EASYNET_REMOTEAPP_SENTINEL_FIXTURE_CMD
                        Same as --sentinel-fixture-cmd.
  EASYNET_REMOTEAPP_SELECTED_SENTINEL_RGB
                        Required by the bundled receiver, formatted as r,g,b.
  EASYNET_REMOTEAPP_UNRELATED_SENTINEL_RGB
                        Required by the bundled receiver, formatted as r,g,b.
  EASYNET_REMOTEAPP_SELECTED_SENTINEL_LABEL
                        Required label for the selected target witness.
  EASYNET_REMOTEAPP_UNRELATED_SENTINEL_LABEL
                        Required label for the unrelated non-target witness.
  EASYNET_REMOTEAPP_TARGET_PID
                        Optional positive process id used by the bundled probe
                        to select an application/window resource by live native
                        identity instead of diagnostic title text.
  EASYNET_REMOTEAPP_CONTROL_DISCOVERY_JSON
                        Optional absolute control.json path. Used only by the
                        bundled EasyNet probe preflight for non-default state
                        directories.

Probe contract:
  The probe command receives:
    EASYNET_REMOTEAPP_FRAME_EVIDENCE_JSON=<path>
    EASYNET_REMOTEAPP_E2E_TARGET_KIND=<window|application>

  It must create the JSON file and prove:
    - live picker used resource.refresh_remote_targets or resource.watch_remote_targets
    - selected resource_ura was used as Invocation.subject
    - remote_desktop.create_session returned a target_binding for that target
    - WebRTC media was decoded into at least one frame
    - decoded frames included selected target content
    - decoded frames excluded unrelated full-display sentinel content
    - scope_audit.scope_widened=false
    - scope_audit.display_fallback_used=false

Bundled probe preflight:
  When --probe-cmd is omitted, the harness uses the bundled EasyNet probe and
  fails before launching host sentinel windows unless daemon control discovery
  publishes daemon_identity. The harness must not synthesize daemon identity
  for the probe because that would invalidate Invocation.subject evidence.

Exit semantics:
  0  evidence is valid, or script was skipped because not enabled.
  1  --run was requested but evidence is missing, malformed, or failed.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --run) RUN=1; shift ;;
    --probe-cmd) PROBE_CMD="${2:?missing value for --probe-cmd}"; shift 2 ;;
    --target-kind)
      case "${2:?missing value for --target-kind}" in
        window|application) TARGET_KIND="$2" ;;
        *) echo "invalid target kind: $2" >&2; exit 64 ;;
      esac
      shift 2
      ;;
    --out-dir) OUT_DIR="${2:?missing value for --out-dir}"; shift 2 ;;
    --sentinel-fixture) SENTINEL_FIXTURE=1; shift ;;
    --sentinel-fixture-cmd)
      SENTINEL_FIXTURE=1
      SENTINEL_FIXTURE_CMD="${2:?missing value for --sentinel-fixture-cmd}"
      shift 2
      ;;
    --self-test) SELF_TEST=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

if [[ -z "$OUT_DIR" ]]; then
  OUT_DIR="$REPO_ROOT/target/e2e/host-remoteapp-decoded-frame/$TIMESTAMP-$TARGET_KIND-$$"
fi

die() {
  echo "[FAIL] $*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "missing command: $1"
}

preflight_bundled_probe_runtime() {
  local control_json="${EASYNET_REMOTEAPP_CONTROL_DISCOVERY_JSON:-}"
  if [[ -z "$control_json" ]]; then
    control_json="$(python3 - <<'PY'
import pathlib
print(pathlib.Path.home() / ".easynet" / "control.json")
PY
)"
  fi

  python3 - "$control_json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
if not path.is_absolute():
    raise SystemExit(
        f"bundled EasyNet host probe requires an absolute control discovery path: {path}"
    )
if not path.exists():
    raise SystemExit(
        "bundled EasyNet host probe requires a running daemon: "
        f"control discovery is missing at {path}"
    )
try:
    discovery = json.loads(path.read_text(encoding="utf-8"))
except Exception as exc:
    raise SystemExit(f"bundled EasyNet host probe cannot parse control discovery {path}: {exc}") from exc

identity = discovery.get("daemon_identity")
if not isinstance(identity, dict):
    raise SystemExit(
        "bundled EasyNet host probe requires daemon_identity in control discovery; "
        "start or restart the daemon before launching host sentinel fixtures"
    )
mode = str(identity.get("mode", "")).strip()
realm = str(identity.get("realm", "")).strip()
node_id = identity.get("node_id")
if mode not in {"device", "both", "hub"}:
    raise SystemExit(f"bundled EasyNet host probe found invalid daemon_identity.mode: {mode!r}")
if not realm:
    raise SystemExit("bundled EasyNet host probe found empty daemon_identity.realm")
if mode in {"device", "both"} and not str(node_id or "").strip():
    raise SystemExit(
        "bundled EasyNet host probe found device-primary daemon_identity without node_id"
    )
PY
}

mkdir -p "$OUT_DIR"
REPORT_JSON="$OUT_DIR/report.json"
REPORT_MD="$OUT_DIR/report.md"
EVIDENCE_JSON="$OUT_DIR/decoded-frame-evidence.json"
SENTINEL_FIXTURE_DIR="$OUT_DIR/sentinel-fixture"

write_skip_report() {
  python3 - "$REPORT_JSON" "$REPORT_MD" <<'PY'
import json
import os
import sys

report = {
    "enabled": False,
    "status": "skipped",
    "reason": "host remoteapp decoded-frame e2e requires --run or EASYNET_HOST_REMOTEAPP_DECODED_FRAME_E2E=1",
}
open(sys.argv[1], "w", encoding="utf-8").write(json.dumps(report, indent=2, sort_keys=True) + "\n")
with open(sys.argv[2], "w", encoding="utf-8") as f:
    f.write("# Host remoteapp decoded-frame E2E report\n\n")
    f.write("- Status: `skipped`\n")
    f.write("- Reason: `host remoteapp decoded-frame e2e requires --run or EASYNET_HOST_REMOTEAPP_DECODED_FRAME_E2E=1`\n")
PY
}

validate_evidence() {
  python3 - "$EVIDENCE_JSON" "$REPORT_JSON" "$REPORT_MD" "$TARGET_KIND" <<'PY'
import json
import os
import sys

evidence_path, report_path, md_path, expected_kind = sys.argv[1:5]
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

selected_resource_ura = get("selected_resource_ura")
invocation_subject_ura = get("invocation.subject_ura")
inventory_ability = get("live_inventory.ability")
target_kind = get("target_binding.target_kind")
target_binding_subject_ura = get("target_binding.subject_ura")
capture_scope = get("target_binding.capture_scope")
binding_id = get("target_binding.binding_id")
binding_epoch = get("target_binding.binding_epoch")
target_identity_epoch = get("target_binding.target_identity_epoch")
target_geometry_revision = get("target_binding.target_geometry_revision")
media_source_epoch = get("target_binding.media_source_epoch")
consent_epoch = get("target_binding.consent_epoch")
resolved_identity = get("target_binding.resolved_identity")
app_window_set = get("target_binding.app_window_set")
scope_widened = get("target_binding.scope_audit.scope_widened")
display_fallback_used = get("target_binding.scope_audit.display_fallback_used")
decoded_frame_count = get("decoded_frames.count")
rtp_packet_count = get("decoded_frames.rtp_packet_count")
transport_kind = get("transport.kind")
production_media_ready = get("production_media_ready")
production_readiness = get("production_readiness")
client_media_ready = get("production_readiness.client_media_ready")
decoded_frame_sample = get("artifacts.decoded_frame_sample")
decoded_width = get("decoded_frames.width")
decoded_height = get("decoded_frames.height")
sentinel_fixture = get("sentinel_fixture")
selected_fixture = get("sentinel_fixture.selected")
unrelated_fixture = get("sentinel_fixture.unrelated")

def parse_rgb_env(name):
    raw = os.environ.get(name, "").strip()
    require(raw, f"{name} must be set so the harness can independently verify decoded pixels")
    if not raw:
        return None
    try:
        parts = [int(part.strip()) for part in raw.split(",")]
    except ValueError:
        require(False, f"{name} must be formatted as r,g,b")
        return None
    require(len(parts) == 3 and all(0 <= part <= 255 for part in parts),
            f"{name} must contain exactly three RGB bytes")
    return parts if len(parts) == 3 else None

def env_int(name, default):
    raw = os.environ.get(name, "").strip()
    if not raw:
        return default
    try:
        value = int(raw)
    except ValueError:
        require(False, f"{name} must be an integer")
        return default
    require(value >= 0, f"{name} must be non-negative")
    return value

def optional_positive_env_int(name):
    raw = os.environ.get(name, "").strip()
    if not raw:
        return None
    try:
        value = int(raw)
    except ValueError:
        require(False, f"{name} must be an integer")
        return None
    require(value > 0, f"{name} must be positive")
    return value

def next_ppm_token(data, offset):
    while True:
        while offset < len(data) and data[offset] in b" \t\r\n":
            offset += 1
        if offset < len(data) and data[offset] == ord("#"):
            while offset < len(data) and data[offset] not in b"\r\n":
                offset += 1
            continue
        break
    start = offset
    while offset < len(data) and data[offset] not in b" \t\r\n":
        offset += 1
    return data[start:offset], offset

def read_ppm_rgb(path):
    with open(path, "rb") as f:
        data = f.read()
    require(data.startswith(b"P6"), "decoded_frame_sample must be a binary PPM (P6) artifact")
    magic, offset = next_ppm_token(data, 0)
    width_token, offset = next_ppm_token(data, offset)
    height_token, offset = next_ppm_token(data, offset)
    max_token, offset = next_ppm_token(data, offset)
    require(magic == b"P6", "decoded_frame_sample PPM magic must be P6")
    try:
        width = int(width_token)
        height = int(height_token)
        max_value = int(max_token)
    except ValueError:
        require(False, "decoded_frame_sample PPM header must contain numeric width, height, and max value")
        return 0, 0, b""
    require(width > 0 and height > 0, "decoded_frame_sample PPM dimensions must be positive")
    require(max_value == 255, "decoded_frame_sample PPM max value must be 255")
    if offset < len(data) and data[offset] in b" \t\r\n":
        offset += 1
    expected_bytes = width * height * 3
    raster = data[offset:]
    require(len(raster) == expected_bytes,
            "decoded_frame_sample PPM raster size must exactly match width*height*3")
    return width, height, raster

def count_rgb_matches(rgb, expected, tolerance):
    if expected is None:
        return 0
    count = 0
    for index in range(0, len(rgb), 3):
        pixel = rgb[index:index + 3]
        if len(pixel) == 3 and all(abs(pixel[channel] - expected[channel]) <= tolerance for channel in range(3)):
            count += 1
    return count

selected_rgb = parse_rgb_env("EASYNET_REMOTEAPP_SELECTED_SENTINEL_RGB")
unrelated_rgb = parse_rgb_env("EASYNET_REMOTEAPP_UNRELATED_SENTINEL_RGB")
selected_pid = optional_positive_env_int("EASYNET_REMOTEAPP_SELECTED_SENTINEL_PID")
unrelated_pid = optional_positive_env_int("EASYNET_REMOTEAPP_UNRELATED_SENTINEL_PID")
sentinel_tolerance = env_int("EASYNET_REMOTEAPP_SENTINEL_TOLERANCE", 64)
selected_min_pixels = env_int("EASYNET_REMOTEAPP_SELECTED_SENTINEL_MIN_PIXELS", 8)

require(isinstance(sentinel_fixture, dict),
        "sentinel_fixture must describe the selected and unrelated visual witnesses")
require(isinstance(selected_fixture, dict),
        "sentinel_fixture.selected must describe the selected target witness")
require(isinstance(unrelated_fixture, dict),
        "sentinel_fixture.unrelated must describe the unrelated non-target witness")
if isinstance(sentinel_fixture, dict):
    require(sentinel_fixture.get("proof") == "dual_target_non_leak",
            "sentinel_fixture.proof must be dual_target_non_leak")
if isinstance(selected_fixture, dict):
    selected_label = selected_fixture.get("label")
    require(isinstance(selected_label, str) and selected_label.strip(),
            "sentinel_fixture.selected.label must be a non-empty string")
    require(selected_fixture.get("resource_ura") == selected_resource_ura,
            "sentinel_fixture.selected.resource_ura must match selected_resource_ura")
    require(selected_fixture.get("target_kind") == expected_kind,
            f"sentinel_fixture.selected.target_kind must be {expected_kind}")
    require(selected_fixture.get("rgb") == selected_rgb,
            "sentinel_fixture.selected.rgb must match EASYNET_REMOTEAPP_SELECTED_SENTINEL_RGB")
    fixture_selected_pid = selected_fixture.get("pid")
    if fixture_selected_pid is not None:
        require(isinstance(fixture_selected_pid, int) and fixture_selected_pid > 0,
                "sentinel_fixture.selected.pid must be a positive integer when present")
    if selected_pid is not None:
        require(fixture_selected_pid == selected_pid,
                "sentinel_fixture.selected.pid must match EASYNET_REMOTEAPP_SELECTED_SENTINEL_PID")
if isinstance(unrelated_fixture, dict):
    unrelated_label = unrelated_fixture.get("label")
    placement = unrelated_fixture.get("placement")
    require(isinstance(unrelated_label, str) and unrelated_label.strip(),
            "sentinel_fixture.unrelated.label must be a non-empty string")
    require(unrelated_fixture.get("rgb") == unrelated_rgb,
            "sentinel_fixture.unrelated.rgb must match EASYNET_REMOTEAPP_UNRELATED_SENTINEL_RGB")
    fixture_unrelated_pid = unrelated_fixture.get("pid")
    if fixture_unrelated_pid is not None:
        require(isinstance(fixture_unrelated_pid, int) and fixture_unrelated_pid > 0,
                "sentinel_fixture.unrelated.pid must be a positive integer when present")
    if unrelated_pid is not None:
        require(fixture_unrelated_pid == unrelated_pid,
                "sentinel_fixture.unrelated.pid must match EASYNET_REMOTEAPP_UNRELATED_SENTINEL_PID")
    require(placement in {"outside_selected_target", "other_window", "other_application", "desktop_background"},
            "sentinel_fixture.unrelated.placement must describe a non-target surface")
    unrelated_resource_ura = unrelated_fixture.get("resource_ura")
    require(not unrelated_resource_ura or unrelated_resource_ura != selected_resource_ura,
            "sentinel_fixture.unrelated.resource_ura must not match selected_resource_ura")
if isinstance(selected_fixture, dict) and isinstance(unrelated_fixture, dict):
    require(selected_fixture.get("label") != unrelated_fixture.get("label"),
            "selected and unrelated sentinel labels must be distinct")

require(evidence.get("status") == "passed", "probe status must be passed")
require(inventory_ability in {"resource.refresh_remote_targets", "resource.watch_remote_targets"},
        "live inventory ability must be resource.refresh_remote_targets or resource.watch_remote_targets")
require(isinstance(selected_resource_ura, str) and selected_resource_ura.startswith("easynet:///"),
        "selected_resource_ura must be an EasyNet URA")
forbidden_address_term = "u" + "ri"
require(forbidden_address_term not in json.dumps(evidence).lower(), "evidence must use URA vocabulary only")
require(invocation_subject_ura == selected_resource_ura,
        "Invocation.subject must equal the selected resource_ura")
require(get("invocation.ability") == "remote_desktop.create_session",
        "invocation ability must be remote_desktop.create_session")
require(target_binding_subject_ura == selected_resource_ura,
        "target_binding.subject_ura must equal the selected resource_ura")
require(target_kind == expected_kind, f"target_binding.target_kind must be {expected_kind}")
require(isinstance(binding_id, str) and binding_id.strip(),
        "target_binding.binding_id must be a non-empty string")
require(isinstance(binding_epoch, int) and binding_epoch > 0,
        "target_binding.binding_epoch must be a positive integer")
require(isinstance(target_identity_epoch, int) and target_identity_epoch > 0,
        "target_binding.target_identity_epoch must be a positive integer")
require(isinstance(target_geometry_revision, int) and target_geometry_revision > 0,
        "target_binding.target_geometry_revision must be a positive integer")
require(isinstance(media_source_epoch, int) and media_source_epoch > 0,
        "target_binding.media_source_epoch must be a positive integer")
require(isinstance(consent_epoch, int) and consent_epoch > 0,
        "target_binding.consent_epoch must be a positive integer")
if expected_kind == "window":
    require(capture_scope == "WindowSurface", "window target must use WindowSurface")
    require(isinstance(resolved_identity, dict),
            "window target must include target_binding.resolved_identity")
    if isinstance(resolved_identity, dict):
        window_id = resolved_identity.get("window_id")
        require(isinstance(window_id, int) and window_id > 0,
                "window resolved_identity.window_id must be a positive integer")
        if selected_pid is not None:
            window_pid = resolved_identity.get("pid") or resolved_identity.get("owner_pid")
            require(window_pid == selected_pid,
                    "window evidence must bind selected sentinel pid to resolved_identity.pid or owner_pid")
else:
    require(capture_scope == "AppSurface", "application target must use AppSurface")
    require(isinstance(app_window_set, dict),
            "application target must include target_binding.app_window_set")
    if isinstance(app_window_set, dict):
        app_display_id = app_window_set.get("display_id")
        app_window_set_epoch = app_window_set.get("window_set_epoch")
        app_resolved_window_ids = app_window_set.get("resolved_window_ids")
        require(isinstance(app_display_id, int) and app_display_id > 0,
                "application app_window_set.display_id must be a positive integer")
        require(isinstance(app_window_set_epoch, int) and app_window_set_epoch > 0,
                "application app_window_set.window_set_epoch must be a positive integer")
        require(isinstance(app_resolved_window_ids, list)
                and len(app_resolved_window_ids) > 0
                and all(isinstance(window_id, int) and window_id > 0 for window_id in app_resolved_window_ids),
                "application app_window_set.resolved_window_ids must be a non-empty positive integer list")
    require(isinstance(resolved_identity, dict),
            "application target must include target_binding.resolved_identity")
    if isinstance(resolved_identity, dict):
        app_identity = resolved_identity.get("app_identity") or resolved_identity.get("bundle_id")
        app_pid = resolved_identity.get("pid")
        require(
            (isinstance(app_identity, str) and app_identity.strip())
            or (isinstance(app_pid, int) and app_pid > 0),
            "application resolved_identity must include app_identity, bundle_id, or positive pid",
        )
        if selected_pid is not None:
            require(
                app_pid == selected_pid
                or (isinstance(app_window_set, dict) and app_window_set.get("primary_pid") == selected_pid),
                "application evidence must bind selected sentinel pid to resolved_identity.pid or app_window_set.primary_pid",
            )
require(scope_widened is False, "scope_audit.scope_widened must be false")
require(display_fallback_used is False, "scope_audit.display_fallback_used must be false")
require(transport_kind == "webrtc", "transport.kind must be webrtc")
require(production_media_ready is True,
        "production_media_ready must be true after the WebRTC production media path is negotiated and sending")
require(isinstance(production_readiness, dict),
        "production_readiness must be present as post-negotiation session evidence")
if isinstance(production_readiness, dict):
    require(production_readiness.get("ready") is True,
            "production_readiness.ready must be true")
    require(production_readiness.get("requires_production_codec") is True,
            "production_readiness.requires_production_codec must be true")
    require(production_readiness.get("production_codec_negotiated") is True,
            "production_readiness.production_codec_negotiated must be true")
    require(production_readiness.get("media_transport_ready") is True,
            "production_readiness.media_transport_ready must be true")
    require(production_readiness.get("client_media_ready") is True,
            "production_readiness.client_media_ready must be true after the receiver reports decoded/presenting media")
require(isinstance(rtp_packet_count, int) and rtp_packet_count > 0,
        "decoded_frames.rtp_packet_count must be a positive integer")
require(isinstance(decoded_frame_count, int) and decoded_frame_count > 0,
        "decoded_frames.count must be a positive integer")
require(isinstance(decoded_width, int) and decoded_width > 0,
        "decoded_frames.width must be a positive integer")
require(isinstance(decoded_height, int) and decoded_height > 0,
        "decoded_frames.height must be a positive integer")
require(get("decoded_frames.selected_content_present") is True,
        "decoded frames must include selected target content")
require(get("decoded_frames.unrelated_sentinel_present") is False,
        "decoded frames must exclude unrelated sentinel content")
require(get("decoded_frames.full_display_leak_detected") is False,
        "decoded frames must not show full-display leakage")
require(decoded_frame_sample,
        "evidence must include a decoded_frame_sample artifact path")
require(isinstance(decoded_frame_sample, str) and os.path.isfile(decoded_frame_sample),
        "decoded_frame_sample artifact must exist on disk")
if isinstance(decoded_frame_sample, str) and os.path.isfile(decoded_frame_sample):
    ppm_width, ppm_height, ppm_rgb = read_ppm_rgb(decoded_frame_sample)
    require(ppm_width == decoded_width and ppm_height == decoded_height,
            "decoded_frame_sample dimensions must match decoded_frames.width/height")
    selected_pixel_count = count_rgb_matches(ppm_rgb, selected_rgb, sentinel_tolerance)
    unrelated_pixel_count = count_rgb_matches(ppm_rgb, unrelated_rgb, sentinel_tolerance)
    require(selected_pixel_count >= selected_min_pixels,
            "decoded_frame_sample pixels must independently contain the selected target sentinel")
    require(unrelated_pixel_count == 0,
            "decoded_frame_sample pixels must independently exclude the unrelated sentinel")
    require(get("decoded_frames.selected_pixel_count") == selected_pixel_count,
            "decoded_frames.selected_pixel_count must match independent artifact scan")
    require(get("decoded_frames.unrelated_pixel_count") == unrelated_pixel_count,
            "decoded_frames.unrelated_pixel_count must match independent artifact scan")
require(get("artifacts.session_id") == get("session_id"),
        "decoded frame artifact session_id must match evidence session_id")
require(get("artifacts.binding_id") == binding_id,
        "decoded frame artifact binding_id must match target_binding.binding_id")
require(get("artifacts.binding_epoch") == binding_epoch,
        "decoded frame artifact binding_epoch must match target_binding.binding_epoch")
require(get("artifacts.capture_scope") == capture_scope,
        "decoded frame artifact capture_scope must match target_binding.capture_scope")

report = {
    "enabled": True,
    "status": "failed" if errors else "passed",
    "target_kind": expected_kind,
    "evidence_json": evidence_path,
    "errors": errors,
    "assertions": {
        "live_inventory": inventory_ability,
        "selected_subject": invocation_subject_ura,
        "target_kind": target_kind,
        "capture_scope": capture_scope,
        "binding_id": binding_id,
        "binding_epoch": binding_epoch,
        "app_window_set": app_window_set,
        "display_fallback_used": display_fallback_used,
        "production_media_ready": production_media_ready,
        "production_readiness": production_readiness,
        "client_media_ready": client_media_ready,
        "rtp_packet_count": rtp_packet_count,
        "decoded_frame_count": decoded_frame_count,
        "decoded_width": decoded_width,
        "decoded_height": decoded_height,
        "selected_content_present": get("decoded_frames.selected_content_present"),
        "unrelated_sentinel_present": get("decoded_frames.unrelated_sentinel_present"),
        "full_display_leak_detected": get("decoded_frames.full_display_leak_detected"),
        "sentinel_fixture": sentinel_fixture,
    },
}
open(report_path, "w", encoding="utf-8").write(json.dumps(report, indent=2, sort_keys=True) + "\n")
with open(md_path, "w", encoding="utf-8") as f:
    f.write("# Host remoteapp decoded-frame E2E report\n\n")
    f.write(f"- Status: `{report['status']}`\n")
    f.write(f"- Target kind: `{expected_kind}`\n")
    f.write(f"- Evidence: `{evidence_path}`\n")
    if errors:
        f.write("\n## Errors\n\n")
        for error in errors:
            f.write(f"- {error}\n")
if errors:
    for error in errors:
        print(error, file=sys.stderr)
    sys.exit(1)
PY
}

if [[ "$SELF_TEST" == "1" ]]; then
  need_cmd python3
  bash -n "$0"
  grep -q "resource.refresh_remote_targets" "$0"
  grep -q "resource.watch_remote_targets" "$0"
  grep -q "remote_desktop.create_session" "$0"
  grep -q "Invocation.subject" "$0"
  grep -q "decoded_frames.unrelated_sentinel_present" "$0"
  grep -q "display_fallback_used" "$0"
  grep -q "production_media_ready" "$0"
  grep -q "production_readiness.production_codec_negotiated" "$0"
  grep -q "production_readiness.client_media_ready" "$0"
  grep -q "host-remoteapp-sentinel-fixture.sh" "$0"
  grep -q "EASYNET_REMOTEAPP_SENTINEL_FIXTURE" "$0"
  grep -q "cleanup.sh" "$0"
  python3 - "$EVIDENCE_JSON" "$TARGET_KIND" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
target_kind = sys.argv[2]
if target_kind not in {"window", "application"}:
    raise SystemExit(f"invalid self-test target kind: {target_kind}")

resource_kind = target_kind
resource_ura = f"easynet:///r/localhost/resource/device.dev/streams/{resource_kind}.test"
capture_scope = "AppSurface" if target_kind == "application" else "WindowSurface"
sample = path.parent / "sample-frame.ppm"
sample.write_bytes(b"P6\n3 3\n255\n" + b"\xff\x00\x00" * 9)

resolved_identity = (
    {
        "app_identity": "com.example.SentinelApp",
        "bundle_id": "com.example.SentinelApp",
        "display_id": 1,
    }
    if target_kind == "application"
    else {"window_id": 7}
)
target_binding = {
    "subject_ura": resource_ura,
    "target_kind": target_kind,
    "capture_scope": capture_scope,
    "binding_id": "binding-test",
    "binding_epoch": 1,
    "target_identity_epoch": 1,
    "target_geometry_revision": 1,
    "media_source_epoch": 1,
    "consent_epoch": 1,
    "resolved_identity": resolved_identity,
    "scope_audit": {
        "scope_widened": False,
        "display_fallback_used": False,
    },
}
if target_kind == "application":
    target_binding["app_window_set"] = {
        "display_id": 1,
        "window_set_epoch": 1,
        "resolved_window_ids": [7],
    }

data = {
    "status": "passed",
    "live_inventory": {"ability": "resource.refresh_remote_targets"},
    "session_id": "rd-self-test",
    "selected_resource_ura": resource_ura,
    "invocation": {
        "ability": "remote_desktop.create_session",
        "subject_ura": resource_ura,
    },
    "target_binding": target_binding,
    "sentinel_fixture": {
        "proof": "dual_target_non_leak",
        "selected": {
            "label": f"selected-{target_kind}-red",
            "resource_ura": resource_ura,
            "rgb": [255, 0, 0],
            "target_kind": target_kind,
        },
        "unrelated": {
            "label": "unrelated-green",
            "placement": "other_application" if target_kind == "application" else "other_window",
            "rgb": [0, 255, 0],
        },
    },
    "transport": {"kind": "webrtc"},
    "production_media_ready": True,
    "production_readiness": {
        "ready": True,
        "requires_production_codec": True,
        "production_codec_negotiated": True,
        "media_transport_ready": True,
        "client_media_ready": True,
    },
    "decoded_frames": {
        "count": 3,
        "rtp_packet_count": 10,
        "width": 3,
        "height": 3,
        "selected_content_present": True,
        "unrelated_sentinel_present": False,
        "full_display_leak_detected": False,
        "selected_pixel_count": 9,
        "unrelated_pixel_count": 0,
    },
    "artifacts": {
        "decoded_frame_sample": str(sample),
        "binding_id": "binding-test",
        "binding_epoch": 1,
        "session_id": "rd-self-test",
        "capture_scope": capture_scope,
    },
}
path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
  export EASYNET_REMOTEAPP_SELECTED_SENTINEL_RGB="255,0,0"
  export EASYNET_REMOTEAPP_UNRELATED_SENTINEL_RGB="0,255,0"
  validate_evidence
  echo "host-remoteapp-decoded-frame-e2e self-test ok"
  exit 0
fi

need_cmd python3

if [[ "$RUN" != "1" ]]; then
  write_skip_report
  echo "SKIP: $REPORT_MD"
  exit 0
fi

if [[ -z "$PROBE_CMD" ]]; then
  [[ -x "$BUNDLED_PROBE" ]] || die "missing executable bundled probe: $BUNDLED_PROBE"
  PROBE_CMD="'$BUNDLED_PROBE'"
  PROBE_CMD_USES_BUNDLED=1
fi

if [[ "$PROBE_CMD_USES_BUNDLED" == "1" ]]; then
  preflight_bundled_probe_runtime
fi

cleanup_sentinel_fixture() {
  if [[ -x "$SENTINEL_FIXTURE_DIR/cleanup.sh" ]]; then
    "$SENTINEL_FIXTURE_DIR/cleanup.sh" >/dev/null 2>&1 || true
  fi
}

if [[ "$SENTINEL_FIXTURE" == "1" ]]; then
  mkdir -p "$SENTINEL_FIXTURE_DIR"
  if [[ -z "$SENTINEL_FIXTURE_CMD" ]]; then
    [[ -x "$BUNDLED_SENTINEL_FIXTURE" ]] || die "missing executable bundled sentinel fixture: $BUNDLED_SENTINEL_FIXTURE"
    SENTINEL_FIXTURE_CMD="'$BUNDLED_SENTINEL_FIXTURE' --target-kind '$TARGET_KIND' --out-dir '$SENTINEL_FIXTURE_DIR'"
  fi
  export EASYNET_REMOTEAPP_SENTINEL_FIXTURE_DIR="$SENTINEL_FIXTURE_DIR"
  export EASYNET_REMOTEAPP_E2E_TARGET_KIND="$TARGET_KIND"
  bash -lc "$SENTINEL_FIXTURE_CMD"
  [[ -s "$SENTINEL_FIXTURE_DIR/env.sh" ]] || die "sentinel fixture did not write env.sh: $SENTINEL_FIXTURE_DIR/env.sh"
  # shellcheck disable=SC1091
  source "$SENTINEL_FIXTURE_DIR/env.sh"
  trap cleanup_sentinel_fixture EXIT
fi

rm -f "$EVIDENCE_JSON"
export EASYNET_REMOTEAPP_FRAME_EVIDENCE_JSON="$EVIDENCE_JSON"
export EASYNET_REMOTEAPP_E2E_TARGET_KIND="$TARGET_KIND"
export EASYNET_REMOTEAPP_SENTINEL_TOLERANCE="${EASYNET_REMOTEAPP_SENTINEL_TOLERANCE:-$DEFAULT_SENTINEL_TOLERANCE}"

bash -lc "$PROBE_CMD"
[[ -s "$EVIDENCE_JSON" ]] || die "probe did not write evidence JSON: $EVIDENCE_JSON"

validate_evidence
echo "PASS: $REPORT_MD"
