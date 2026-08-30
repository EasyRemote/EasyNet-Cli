#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
SPEC="$ROOT/docs/design/remoteapp-targeted-session-spec.md"
AUDIT="$ROOT/docs/design/remoteapp-product-readiness-audit-2026-08-22.md"
MATRIX="$ROOT/docs/design/remoteapp-product-readiness-matrix.json"
PLAN="$ROOT/pr/20260822-remoteapp-product-closure/02-evidence-audit.md"
CROSS_DEVICE_SMOKE="$ROOT/tools/scripts/remoteapp-cross-device-product-smoke.sh"
CROSS_DEVICE_REMOTEAPP="$ROOT/tools/scripts/remoteapp-cross-device-remoteapp-e2e.sh"
PRODUCT_COMPLETION="$ROOT/tools/scripts/remoteapp-product-completion-e2e.sh"
PRODUCT_FINALIZER="$ROOT/tools/scripts/remoteapp-product-finalize.py"
EVIDENCE_PROVENANCE="$ROOT/tools/scripts/remoteapp-evidence-provenance.py"
ATTESTATION_TRUST_ADMIN="$ROOT/tools/scripts/remoteapp-attestation-trust.py"
RECEIPT_VERIFICATION="$ROOT/src/cli/commands/receipt_verification.rs"
MAIN_CRATE_IMPL_TESTS="$ROOT/tools/scripts/check-remoteapp-main-crate-implementation-tests.sh"
CAPTURE_MATRIX="$ROOT/tools/scripts/remoteapp-cross-platform-capture-e2e.sh"
INPUT_INJECTION="$ROOT/tools/scripts/remoteapp-input-injection-e2e.sh"
TARGET_INPUT_RUNNER="$ROOT/tools/scripts/host-remoteapp-target-input-e2e.sh"
MEDIA_ADAPTATION="$ROOT/tools/scripts/remoteapp-media-adaptation-e2e.sh"
MEDIA_ADAPTATION_RUNNER="$ROOT/tools/scripts/host-remoteapp-media-adaptation-e2e.sh"
MULTI_WINDOW_TRACKING="$ROOT/tools/scripts/remoteapp-multi-window-tracking-e2e.sh"
NETWORK_FALLBACK="$ROOT/tools/scripts/remoteapp-network-fallback-e2e.sh"
DIRECT_ROUTE_RUNNER="$ROOT/tools/scripts/host-remoteapp-direct-e2e.sh"
TURN_RELAY_RUNNER="$ROOT/tools/scripts/host-remoteapp-turn-relay-e2e.sh"
EASYNET_RELAY_RUNNER="$ROOT/tools/scripts/host-remoteapp-easynet-relay-e2e.sh"
EASYNET_RELAY_REFRESH_VERIFIER="$ROOT/tools/scripts/verify-remoteapp-relay-refresh.py"
EASYNET_RELAY_RUNNER_TEST="$ROOT/tests/scripts/test_host_remoteapp_easynet_relay_e2e.sh"
STUN_SRFLX_RUNNER="$ROOT/tools/scripts/host-remoteapp-stun-srflx-e2e.sh"
STUN_BINDING_SERVER="$ROOT/tools/scripts/remoteapp-stun-binding-server.py"
NETWORK_SCENARIO_PROJECTOR="$ROOT/tools/scripts/project-remoteapp-network-scenario.py"
FRONTEND_PRODUCT_FLOW="$ROOT/tools/scripts/frontend-remoteapp-product-flow-e2e.sh"
FRONTEND_BROWSER_LIFECYCLE="$ROOT/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh"
PERMISSION_SUBJECT="$ROOT/tools/scripts/host-remoteapp-permission-subject-e2e.sh"
TARGET_FRESHNESS="$ROOT/tools/scripts/host-remoteapp-target-picker-freshness-e2e.sh"
TARGET_SELECTOR="$ROOT/tools/scripts/remoteapp-select-live-target.py"
DECODED_FRAME="$ROOT/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
DECODED_FRAME_PROBE="$ROOT/tools/scripts/host-remoteapp-decoded-frame-probe.sh"
VIEW_ONLY_INPUT_SAFETY="$ROOT/tools/scripts/host-remoteapp-view-only-input-safety-e2e.sh"
SESSION_TIMEOUT="$ROOT/tools/scripts/host-remoteapp-session-timeout-e2e.sh"
SESSION_CANCEL="$ROOT/tools/scripts/host-remoteapp-session-cancel-e2e.sh"
PERMISSION_REVOKE="$ROOT/tools/scripts/host-remoteapp-permission-revoke-e2e.sh"
SESSION_RESUME="$ROOT/tools/scripts/host-remoteapp-session-resume-e2e.sh"
CRASH_RESTART_RECOVERY="$ROOT/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh"
TARGET_MONITOR_WORKER_RECOVERY="$ROOT/tools/scripts/host-remoteapp-target-monitor-worker-recovery-e2e.sh"
TARGET_MONITOR_WORKER_RECOVERY_TEST="$ROOT/tests/scripts/test_host_remoteapp_target_monitor_worker_recovery_e2e.sh"
LIFECYCLE_HARNESS_LIB="$ROOT/tools/scripts/remoteapp-lifecycle-harness-lib.sh"
SESSION="$ROOT/plugins/remote-desktop/src/session.rs"
SESSION_EVENTS="$ROOT/plugins/remote-desktop/src/session_events.rs"
HOSTED_WEBRTC_MEDIA="$ROOT/plugins/remote-desktop/src/transport/webrtc_hosted_media.rs"
REMOTE_DESKTOP_MEDIA="$ROOT/plugins/remote-desktop/src/media/mod.rs"
REMOTE_DESKTOP_NETWORK="$ROOT/plugins/remote-desktop/src/network.rs"
REMOTE_DESKTOP_RELAY_LEASE="$ROOT/plugins/remote-desktop/src/relay_lease.rs"
REMOTE_DESKTOP_LEASE_MONITOR="$ROOT/plugins/remote-desktop/src/lease_monitor.rs"
REMOTE_DESKTOP_EMBEDDED="$ROOT/plugins/remote-desktop/src/embedded.rs"
REMOTE_DESKTOP_VIEW_TRANSPORT="$ROOT/plugins/remote-desktop/src/view_transport.rs"
DAEMON_REMOTEAPP_RELAY="$ROOT/src/daemon/plugins/remoteapp_relay.rs"
DAEMON_PLUGINS="$ROOT/src/daemon/plugins/mod.rs"
WEBRTC_ENCODED_AUDIO="$ROOT/plugins/remote-desktop/src/transport/webrtc_encoded_audio.rs"
MEDIA_HOST_LIB="$ROOT/plugins/remote-desktop/media-host/src/lib.rs"
MEDIA_HOST_MAC="$ROOT/plugins/remote-desktop/media-host/src/macos_sck.rs"
MEDIA_HOST_MAC_AUDIO="$ROOT/plugins/remote-desktop/media-host/src/macos_audio.rs"
MEDIA_HOST_PROTOCOL="$ROOT/plugins/remote-desktop/native-protocol/src/media_session.rs"
SHARED_MEDIA_LANE="$ROOT/plugins/remote-desktop/native-protocol/src/shared_media_lane.rs"
WEBRTC_ENDPOINT="$ROOT/plugins/remote-desktop/src/transport/webrtc_endpoint.rs"
WEBRTC_CALLBACKS="$ROOT/plugins/remote-desktop/src/transport/webrtc.rs"
TRANSPORT_MANAGER="$ROOT/plugins/remote-desktop/src/transport/manager.rs"
INVOKE_BIDI="$ROOT/plugins/remote-desktop/src/invoke_bidi.rs"
BIDI_TERMINAL="$ROOT/plugins/remote-desktop/src/transport/terminal.rs"
TARGET_TRACKING="$ROOT/plugins/remote-desktop/src/target_tracking.rs"
SESSION_RECOVERY="$ROOT/plugins/remote-desktop/src/session_recovery.rs"
SESSION_STATE="$ROOT/plugins/remote-desktop/src/session_state.rs"
SESSION_STORE="$ROOT/plugins/remote-desktop/src/session_store.rs"
SESSION_LIFECYCLE="$ROOT/plugins/remote-desktop/src/session_lifecycle.rs"
RUNTIME="$ROOT/plugins/remote-desktop/src/runtime.rs"
SESSION_VIEW="$ROOT/plugins/remote-desktop/src/view.rs"
SESSION_HANDLERS="$ROOT/plugins/remote-desktop/src/handlers/mod.rs"
REPORT_CLIENT_STATE_HANDLER="$ROOT/plugins/remote-desktop/src/handlers/report_client_state.rs"
REPORT_CLIENT_STATE_DESCRIPTOR="$ROOT/plugins/remote-desktop/abilities/remote_desktop.report_client_state.ability.toml"
CREATE_SESSION_HANDLER="$ROOT/plugins/remote-desktop/src/handlers/create_session.rs"
REFRESH_LEASE_HANDLER="$ROOT/plugins/remote-desktop/src/handlers/refresh_lease.rs"
SHOW_SESSION_HANDLER="$ROOT/plugins/remote-desktop/src/handlers/show_session.rs"
END_SESSION_HANDLER="$ROOT/plugins/remote-desktop/src/handlers/end_session.rs"
ATTACH_HANDLER="$ROOT/plugins/remote-desktop/src/handlers/attach.rs"
REMOTE_DESKTOP_SCHEMA="$ROOT/plugins/remote-desktop/src/schema.rs"
SESSION_TRANSPORT_STATE="$ROOT/plugins/remote-desktop/src/session_transport_state.rs"
HOST_AUDIO_CAPABILITY="$ROOT/plugins/remote-desktop/src/media/host_audio_capability.rs"
EVENT_LOG="$ROOT/plugins/remote-desktop/src/event_log.rs"
TARGET_MONITOR="$ROOT/plugins/remote-desktop/src/target_monitor.rs"
TARGET_SNAPSHOT="$ROOT/plugins/remote-desktop/src/target_snapshot.rs"
INPUT="$ROOT/plugins/remote-desktop/src/input.rs"
REMOTE_DESKTOP_PLUGIN_MANIFEST="$ROOT/plugins/remote-desktop/plugin.toml"
REMOTE_DESKTOP_REGISTRATION="$ROOT/plugins/remote-desktop/src/registration.rs"
PLUGIN_SURFACE="$ROOT/src/daemon/plugins/surface.rs"
REAL_INVOKE_TESTS="$ROOT/src/daemon/ability/builtins/real_invoke_tests.rs"

fail() {
  printf 'check-remoteapp-product-closure-audit: %s\n' "$1" >&2
  exit 1
}

require() {
  local pattern="$1"
  local path="$2"
  local message="$3"
  rg -q -- "$pattern" "$path" || fail "$message"
}

reject() {
  local pattern="$1"
  local path="$2"
  local message="$3"
  if rg -q -- "$pattern" "$path"; then
    fail "$message"
  fi
}

[[ -f "$SPEC" ]] || fail "missing RemoteApp targeted-session SPEC"
[[ -f "$AUDIT" ]] || fail "missing RemoteApp product readiness audit"
[[ -f "$MATRIX" ]] || fail "missing RemoteApp product readiness matrix"
[[ -f "$TRANSPORT_MANAGER" ]] || fail "missing RemoteApp transport manager"
[[ -f "$WEBRTC_CALLBACKS" ]] || fail "missing RemoteApp WebRTC callback adapter"
[[ -f "$INVOKE_BIDI" ]] || fail "missing RemoteApp diagnostic preview worker"
[[ -f "$BIDI_TERMINAL" ]] || fail "missing RemoteApp Bidi terminal-frame guard"
[[ -f "$ATTACH_HANDLER" ]] || fail "missing RemoteApp attach handler"
[[ -f "$PLAN" ]] || fail "missing RemoteApp product closure evidence plan"
[[ -f "$CROSS_DEVICE_SMOKE" ]] || fail "missing RemoteApp cross-device product smoke gate"
[[ -f "$CROSS_DEVICE_REMOTEAPP" ]] || fail "missing RemoteApp cross-device RemoteApp verifier"
[[ -f "$PRODUCT_COMPLETION" ]] || fail "missing RemoteApp product-completion evidence gate"
[[ -x "$PRODUCT_COMPLETION" ]] || fail "RemoteApp product-completion evidence gate must be executable"
[[ -x "$PRODUCT_FINALIZER" ]] || fail "missing executable RemoteApp product-completion finalizer"
[[ -f "$EVIDENCE_PROVENANCE" ]] || fail "missing RemoteApp signed evidence verifier"
[[ -f "$RECEIPT_VERIFICATION" ]] || fail "missing Axon finalization proof verifier"
[[ -f "$EVIDENCE_PROVENANCE" ]] || fail "missing RemoteApp signed evidence provenance verifier"
[[ -f "$EVIDENCE_PROVENANCE" ]] || fail "missing RemoteApp evidence provenance boundary"
[[ -f "$MAIN_CRATE_IMPL_TESTS" ]] || fail "missing RemoteApp main-crate implementation test gate"
[[ -f "$CAPTURE_MATRIX" ]] || fail "missing RemoteApp cross-platform capture verifier"
[[ -f "$INPUT_INJECTION" ]] || fail "missing RemoteApp input injection verifier"
[[ -x "$TARGET_INPUT_RUNNER" ]] || fail "missing executable RemoteApp target-local input host runner"
[[ -x "$MEDIA_ADAPTATION_RUNNER" ]] || fail "missing executable RemoteApp media adaptation host runner"
[[ -f "$MEDIA_ADAPTATION" ]] || fail "missing RemoteApp media adaptation evidence verifier"
[[ -f "$MULTI_WINDOW_TRACKING" ]] || fail "missing RemoteApp multi-window tracking evidence verifier"
[[ -f "$NETWORK_FALLBACK" ]] || fail "missing RemoteApp network fallback evidence verifier"
[[ -x "$DIRECT_ROUTE_RUNNER" ]] || fail "missing executable RemoteApp direct-route host runner"
[[ -x "$TURN_RELAY_RUNNER" ]] || fail "missing executable RemoteApp TURN relay host runner"
[[ -x "$EASYNET_RELAY_RUNNER" ]] || fail "missing executable RemoteApp EasyNet relay host runner"
[[ -x "$EASYNET_RELAY_REFRESH_VERIFIER" ]] || fail "missing executable RemoteApp EasyNet relay refresh verifier"
[[ -x "$EASYNET_RELAY_RUNNER_TEST" ]] || fail "missing executable RemoteApp EasyNet relay refresh integration test"
[[ -x "$STUN_SRFLX_RUNNER" ]] || fail "missing executable RemoteApp STUN srflx host runner"
[[ -x "$STUN_BINDING_SERVER" ]] || fail "missing executable bounded RemoteApp STUN binding fixture"
[[ -x "$NETWORK_SCENARIO_PROJECTOR" ]] || fail "missing executable RemoteApp network scenario projector"
[[ -f "$FRONTEND_PRODUCT_FLOW" ]] || fail "missing RemoteApp frontend product-flow verifier"
[[ -f "$FRONTEND_BROWSER_LIFECYCLE" ]] || fail "missing RemoteApp frontend Browser/Tauri lifecycle verifier"
[[ -f "$PERMISSION_SUBJECT" ]] || fail "missing RemoteApp host permission-subject verifier"
[[ -f "$TARGET_FRESHNESS" ]] || fail "missing RemoteApp host target-picker freshness verifier"
[[ -f "$TARGET_SELECTOR" ]] || fail "missing canonical RemoteApp live target selector"
[[ -f "$DECODED_FRAME" ]] || fail "missing RemoteApp host decoded-frame verifier"
[[ -f "$VIEW_ONLY_INPUT_SAFETY" ]] || fail "missing RemoteApp host view-only input safety verifier"
[[ -f "$SESSION_TIMEOUT" ]] || fail "missing RemoteApp session timeout E2E harness"
[[ -f "$SESSION_CANCEL" ]] || fail "missing RemoteApp session cancel E2E harness"
[[ -f "$PERMISSION_REVOKE" ]] || fail "missing RemoteApp permission revoke E2E harness"
[[ -f "$SESSION_RESUME" ]] || fail "missing RemoteApp session resume E2E harness"
[[ -f "$CRASH_RESTART_RECOVERY" ]] || fail "missing RemoteApp crash/restart recovery evidence verifier"
[[ -f "$LIFECYCLE_HARNESS_LIB" ]] || fail "missing RemoteApp lifecycle harness helper library"
[[ -f "$SESSION" ]] || fail "missing RemoteApp session aggregate"
[[ -f "$HOSTED_WEBRTC_MEDIA" ]] || fail "missing RemoteApp hosted WebRTC media bridge"
[[ -f "$REMOTE_DESKTOP_MEDIA" ]] || fail "missing RemoteApp media contract module"
[[ -f "$REMOTE_DESKTOP_RELAY_LEASE" ]] || fail "missing RemoteApp Hub relay lease port"
[[ -f "$REMOTE_DESKTOP_LEASE_MONITOR" ]] || fail "missing RemoteApp lease monitor"
[[ -f "$REMOTE_DESKTOP_EMBEDDED" ]] || fail "missing RemoteApp embedded provider"
[[ -f "$REMOTE_DESKTOP_VIEW_TRANSPORT" ]] || fail "missing RemoteApp transport view projection"
[[ -f "$DAEMON_REMOTEAPP_RELAY" ]] || fail "missing daemon Hub relay lease adapter"
[[ -f "$DAEMON_PLUGINS" ]] || fail "missing daemon plugin composition root"
[[ -f "$WEBRTC_ENCODED_AUDIO" ]] || fail "missing RemoteApp bounded encoded-audio WebRTC writer"
[[ -f "$MEDIA_HOST_LIB" ]] || fail "missing canonical RemoteApp media-host implementation"
[[ -f "$MEDIA_HOST_MAC" ]] || fail "missing hosted ScreenCaptureKit capture implementation"
[[ -f "$MEDIA_HOST_MAC_AUDIO" ]] || fail "missing hosted ScreenCaptureKit Opus implementation"
[[ -f "$MEDIA_HOST_PROTOCOL" ]] || fail "missing RemoteApp media-host protocol"
[[ -f "$SHARED_MEDIA_LANE" ]] || fail "missing RemoteApp shared media lane"
for obsolete_media in \
  "$ROOT/plugins/remote-desktop/src/transport/webrtc_native_media.rs" \
  "$ROOT/plugins/remote-desktop/src/media/native.rs" \
  "$ROOT/plugins/remote-desktop/src/screencapturekit_audio.rs" \
  "$ROOT/plugins/remote-desktop/src/screencapturekit_capture.rs"; do
  [[ ! -e "$obsolete_media" ]] || fail "obsolete daemon-local media implementation remains: $obsolete_media"
done
[[ -f "$WEBRTC_ENDPOINT" ]] || fail "missing RemoteApp WebRTC endpoint"
[[ -f "$TARGET_TRACKING" ]] || fail "missing RemoteApp target tracking state machine"
[[ -f "$SESSION_RECOVERY" ]] || fail "missing RemoteApp session recovery snapshot store"
[[ -f "$SESSION_STATE" ]] || fail "missing RemoteApp session lifecycle state machine"
[[ -f "$SESSION_LIFECYCLE" ]] || fail "missing RemoteApp session lifecycle module"
[[ -f "$RUNTIME" ]] || fail "missing RemoteApp runtime module"
[[ -f "$SESSION_VIEW" ]] || fail "missing RemoteApp session view projection"
[[ -f "$SESSION_HANDLERS" ]] || fail "missing RemoteApp session handler tests"
[[ -f "$REPORT_CLIENT_STATE_HANDLER" ]] || fail "missing RemoteApp report_client_state handler"
[[ -f "$CREATE_SESSION_HANDLER" ]] || fail "missing RemoteApp create_session handler"
[[ -f "$REFRESH_LEASE_HANDLER" ]] || fail "missing RemoteApp refresh_lease handler"
[[ -f "$SHOW_SESSION_HANDLER" ]] || fail "missing RemoteApp show_session handler"
[[ -f "$END_SESSION_HANDLER" ]] || fail "missing RemoteApp end_session handler"
[[ -f "$REMOTE_DESKTOP_SCHEMA" ]] || fail "missing RemoteApp ability schemas"
[[ -f "$EVENT_LOG" ]] || fail "missing RemoteApp event log"
[[ -f "$TARGET_MONITOR" ]] || fail "missing RemoteApp target monitor"
[[ -f "$TARGET_SNAPSHOT" ]] || fail "missing RemoteApp target snapshot executor"
[[ -f "$INPUT" ]] || fail "missing RemoteApp input execution plane"
[[ -f "$REMOTE_DESKTOP_PLUGIN_MANIFEST" ]] || fail "missing RemoteApp plugin manifest"
[[ -f "$REMOTE_DESKTOP_REGISTRATION" ]] || fail "missing RemoteApp compiled registration"
[[ -f "$PLUGIN_SURFACE" ]] || fail "missing plugin operator surface projection"
[[ -f "$REAL_INVOKE_TESTS" ]] || fail "missing real invoke tests"
[[ -f "$TARGET_MONITOR_WORKER_RECOVERY" ]] || fail "missing RemoteApp target-monitor worker recovery live runner"
[[ -f "$TARGET_MONITOR_WORKER_RECOVERY_TEST" ]] || fail "missing RemoteApp target-monitor worker recovery contract test"

for lifecycle_harness in "$SESSION_TIMEOUT" "$SESSION_CANCEL" "$SESSION_RESUME"; do
  require 'remoteapp-lifecycle-harness-lib\.sh' "$lifecycle_harness" \
    "RemoteApp lifecycle harness must share lifecycle helper logic: $lifecycle_harness"
  require 'ABILITY_CATALOG_JSON' "$lifecycle_harness" \
    "RemoteApp lifecycle harness must persist ability catalog evidence: $lifecycle_harness"
  require 'ability list --format json' "$lifecycle_harness" \
    "RemoteApp lifecycle harness must resolve public Ability URAs from the committed catalog: $lifecycle_harness"
  require 'remoteapp_resolve_rpc_ability_ura' "$lifecycle_harness" \
    "RemoteApp lifecycle harness must require exactly one rpc Ability URA: $lifecycle_harness"
  require 'remoteapp_session_approval_causal_context_json' "$lifecycle_harness" \
    "RemoteApp lifecycle harness must use the session approval receipt as causal context: $lifecycle_harness"
  require '--causal-context-json "\$SESSION_CAUSAL_CONTEXT_JSON"' "$lifecycle_harness" \
    "RemoteApp lifecycle harness must pass the session approval receipt causal context: $lifecycle_harness"
  require 'ability invoke "\$END_SESSION_ABILITY_URA"' "$lifecycle_harness" \
    "RemoteApp lifecycle harness must invoke end_session by full Ability URA: $lifecycle_harness"
  reject '--causal-root' "$lifecycle_harness" \
    "RemoteApp lifecycle harness must not use root causal context for session lifecycle calls: $lifecycle_harness"
  reject 'ability invoke remote_desktop\.end_session' "$lifecycle_harness" \
    "RemoteApp lifecycle harness must not invoke end_session through short ability name: $lifecycle_harness"
done

require 'REFRESH_LEASE_ABILITY_URA' "$SESSION_RESUME" \
  'RemoteApp resume harness must resolve refresh_lease through a full Ability URA'
require 'ability invoke "\$REFRESH_LEASE_ABILITY_URA"' "$SESSION_RESUME" \
  'RemoteApp resume harness must invoke refresh_lease by full Ability URA'
reject 'ability invoke remote_desktop\.refresh_lease' "$SESSION_RESUME" \
  'RemoteApp resume harness must not invoke refresh_lease through short ability name'

bash "$PRODUCT_COMPLETION" --self-test >/dev/null
python3 -m py_compile "$EVIDENCE_PROVENANCE" "$PRODUCT_FINALIZER"

for provenance_verifier in \
  "$FRONTEND_BROWSER_LIFECYCLE" \
  "$CROSS_DEVICE_REMOTEAPP" \
  "$CAPTURE_MATRIX" \
  "$INPUT_INJECTION" \
  "$MEDIA_ADAPTATION" \
  "$MULTI_WINDOW_TRACKING" \
  "$NETWORK_FALLBACK" \
  "$PERMISSION_SUBJECT" \
  "$TARGET_FRESHNESS" \
  "$DECODED_FRAME" \
  "$VIEW_ONLY_INPUT_SAFETY" \
  "$SESSION_TIMEOUT" \
  "$SESSION_CANCEL" \
  "$PERMISSION_REVOKE" \
  "$SESSION_RESUME" \
  "$CRASH_RESTART_RECOVERY"; do
  require 'remoteapp-evidence-provenance.py' "$provenance_verifier" \
    "RemoteApp verifier must use the shared evidence provenance boundary: $provenance_verifier"
  require 'project-report' "$provenance_verifier" \
    "RemoteApp verifier must project verified provenance into its report: $provenance_verifier"
done
require 'evidence_origin.*live_runner' "$FRONTEND_PRODUCT_FLOW" \
  'frontend product flow must identify live report and step artifacts'
require 'evidence_origin.*live_runner' "$CROSS_DEVICE_SMOKE" \
  'cross-device product smoke must identify completed live evidence'
require 'evidence_origin.*live_runner' "$TARGET_INPUT_RUNNER" \
  'target input runner must identify completed live evidence'

require 'remoteapp_resolve_rpc_ability_ura' "$LIFECYCLE_HARNESS_LIB" \
  'RemoteApp lifecycle helper must implement catalog Ability URA resolution'
require 'remoteapp_session_approval_causal_context_json' "$LIFECYCLE_HARNESS_LIB" \
  'RemoteApp lifecycle helper must implement approval receipt causal context projection'
require 'bidi_wire_kind = "metadata_json_plus_binary"' "$REMOTE_DESKTOP_PLUGIN_MANIFEST" \
  'RemoteApp attach manifest must declare metadata_json_plus_binary bidi media framing'
reject 'bidi_wire_kind = "json_frames"' "$REMOTE_DESKTOP_PLUGIN_MANIFEST" \
  'RemoteApp attach manifest must not regress to JSON-only bidi framing'
require 'PluginBidiWireKind::MetadataJsonPlusBinary' "$REMOTE_DESKTOP_REGISTRATION" \
  'RemoteApp compiled attach spec must declare metadata/binary bidi framing'
reject 'PluginBidiWireKind::JsonFrames' "$REMOTE_DESKTOP_REGISTRATION" \
  'RemoteApp compiled attach spec must not regress to JSON-only bidi framing'
require 'bidi_wire_kind: ability\.bidi_wire_kind\(\)\.map\(Into::into\)' "$PLUGIN_SURFACE" \
  'plugin surface must project declared bidi wire kind for frontend/catalog discovery'
require 'PluginBidiWireKindView::MetadataJsonPlusBinary' "$PLUGIN_SURFACE" \
  'plugin surface must expose metadata_json_plus_binary bidi framing'
require 'plugin_host_surface_projects_declared_bidi_wire_kind' "$PLUGIN_SURFACE" \
  'plugin surface must test declared bidi wire-kind projection'
require 'real_device_plugin_status_surfaces_remoteapp_attach_wire_kind' "$REAL_INVOKE_TESTS" \
  'real plugin.status test must assert RemoteApp attach wire-kind projection'
require 'metadata_json_plus_binary' "$REAL_INVOKE_TESTS" \
  'real plugin.status test must assert metadata_json_plus_binary for RemoteApp attach'

python3 - "$MATRIX" <<'PY' || fail "RemoteApp product readiness matrix is invalid"
import json
import sys

path = sys.argv[1]
matrix = json.load(open(path, encoding="utf-8"))

required_ids = {
    "application_window_capture",
    "input_injection",
    "audio_video_adaptation",
    "multi_window_tracking",
    "session_recovery_lifecycle",
    "network_fallback",
    "frontend_lifecycle",
    "cross_device_e2e",
}
allowed_statuses = {"incomplete", "partial"}

errors = []
if matrix.get("schema_version") != 1:
    errors.append("schema_version must be 1")
if matrix.get("status") != "incomplete":
    errors.append("matrix status must remain incomplete")
if matrix.get("product_complete") is not False:
    errors.append("product_complete must be false until every row is proven")

requirements = matrix.get("requirements")
if not isinstance(requirements, list):
    errors.append("requirements must be a list")
    requirements = []

seen = {row.get("id") for row in requirements if isinstance(row, dict)}
missing = sorted(required_ids - seen)
extra = sorted(seen - required_ids)
if missing:
    errors.append("missing requirement ids: " + ", ".join(missing))
if extra:
    errors.append("unexpected requirement ids: " + ", ".join(extra))

for row in requirements:
    if not isinstance(row, dict):
        errors.append("requirement rows must be objects")
        continue
    row_id = row.get("id", "<missing>")
    status = row.get("status")
    if status not in allowed_statuses:
        errors.append(f"{row_id}: status must be one of {sorted(allowed_statuses)}")
    for key in (
        "requirement",
        "current_evidence",
        "required_evidence_before_product_complete",
        "non_claims",
    ):
        value = row.get(key)
        if isinstance(value, str):
            ok = bool(value.strip())
        elif isinstance(value, list):
            ok = bool(value) and all(isinstance(item, str) and item.strip() for item in value)
        else:
            ok = False
        if not ok:
            errors.append(f"{row_id}: {key} must be non-empty")

if errors:
    for error in errors:
        print(error, file=sys.stderr)
    raise SystemExit(1)
PY

reject 'full acceptance verified' "$SPEC" \
  'targeted-session SPEC must not claim full product acceptance'
require 'full RemoteApp product closure incomplete' "$SPEC" \
  'targeted-session SPEC must state that full RemoteApp product closure is incomplete'
require 'macOS interactive app/window input uses `target_local` only after explicit' "$SPEC" \
  'SPEC must bind target-local input to explicit consent and host execution guards'
require 'Clipboard and file-drop frame types exist in the input model but are not implemented' "$SPEC" \
  'SPEC must retain unsupported clipboard/file-drop boundary'
require 'MultiAppSurface' "$SPEC" \
  'SPEC must retain multi-display application capture limitation'

require 'Status: product closure incomplete' "$AUDIT" \
  'audit must explicitly mark product closure incomplete'
require 'Passing the current boundary gates' "$AUDIT" \
  'audit must name current boundary gates'
require 'does not mean RemoteApp is product-complete' "$AUDIT" \
  'audit must distinguish boundary gates from product completion'
require 'Application/window selection and stable capture across macOS/Windows/Linux' "$AUDIT" \
  'audit must cover cross-platform application/window/display capture'
require 'remoteapp-cross-platform-capture-e2e\.sh' "$AUDIT" \
  'audit must record the cross-platform capture evidence verifier'
require 'Mouse/keyboard input injection is controllable' "$AUDIT" \
  'audit must cover product input injection'
require 'remoteapp-input-injection-e2e\.sh' "$AUDIT" \
  'audit must record the input injection evidence verifier'
require 'Audio/video codec, frame rate, bitrate adaptation' "$AUDIT" \
  'audit must cover media codec/adaptation'
require 'remoteapp-media-adaptation-e2e\.sh' "$AUDIT" \
  'audit must record the media adaptation evidence verifier'
require 'host-remoteapp-media-adaptation-e2e\.sh' "$AUDIT" \
  'audit must record the executable media adaptation host runner'
require 'Multi-window/multi-application independent tracking' "$AUDIT" \
  'audit must cover multi-window/application tracking as execution effect'
require 'remoteapp-multi-window-tracking-e2e\.sh' "$AUDIT" \
  'audit must record the multi-window tracking evidence verifier'
require 'Disconnect/reconnect, session resume, consent revoke, cancel, timeout' "$AUDIT" \
  'audit must cover recovery and lifecycle closure'
require 'NAT/relay/WebRTC/direct fallback network paths' "$AUDIT" \
  'audit must cover real network paths'
require 'remoteapp-network-fallback-e2e\.sh' "$AUDIT" \
  'audit must record the network fallback evidence verifier'
require 'Frontend UI can discover, authorize, start, display, control, and end session' "$AUDIT" \
  'audit must cover frontend full lifecycle'
require 'frontend-remoteapp-browser-lifecycle-e2e\.sh' "$AUDIT" \
  'audit must record the Browser/Tauri lifecycle evidence verifier'
require 'submitted data-channel frame and daemon applied event target_focus_epoch' "$AUDIT" \
  'audit must record frontend input focus-epoch product evidence semantics'
require 'Cross-device E2E smoke/regression exists beyond local provider boundary' "$AUDIT" \
  'audit must cover cross-device proof'
require 'remoteapp-cross-device-product-smoke.sh' "$AUDIT" \
  'audit must name the cross-device product smoke gate'
require 'remoteapp-product-completion-e2e.sh' "$AUDIT" \
  'audit must name the product-completion evidence gate'
require 'remoteapp-product-finalize.py' "$AUDIT" \
  'audit must name the independent product-completion finalizer'
require 'frontend product-flow, Browser/Tauri' "$AUDIT" \
  'audit must summarize product-completion required reports'
require 'stable `script` identity' "$AUDIT" \
  'audit must record product-completion report provenance validation'
require 'target_kind=both' "$AUDIT" \
  'audit must record that product-completion requires both window and application target coverage'
require 'target-narrowed' "$AUDIT" \
  'audit must reject target-narrowed evidence as product completion'
require 'traceable `result.json` step artifacts' "$AUDIT" \
  'audit must record product-flow step artifact traceability'
require 'check-remoteapp-main-crate-implementation-tests.sh' "$AUDIT" \
  'audit must name the main-crate implementation test gate'
require 'production_target_subjects' "$AUDIT" \
  'audit must record production target subject projection semantics'
require 'diagnostic_target_subjects' "$AUDIT" \
  'audit must record diagnostic target subject projection semantics'
require 'platform_support' "$AUDIT" \
  'audit must record platform support projection semantics'
require 'input_control_support' "$AUDIT" \
  'audit must record input control support projection semantics'
require 'media_pipeline_support' "$AUDIT" \
  'audit must record media pipeline support projection semantics'
require 'audio/video scope' "$AUDIT" \
  'audit must record conditional audio/video media pipeline scope'
require 'missing media-adaptation E2E as a product blocker' "$AUDIT" \
  'audit must record missing media-adaptation E2E as a product blocker'
require 'EASYNET_REMOTEAPP_MEDIA_DEGRADED_NETWORK_APPLY_COMMAND' "$MEDIA_ADAPTATION_RUNNER" \
  'media adaptation host runner must require an explicit degraded-network fixture'
require 'EASYNET_REMOTEAPP_MEDIA_BASELINE_RESET_COMMAND' "$MEDIA_ADAPTATION_RUNNER" \
  'media adaptation host runner must require an explicit baseline reset'
require 'EASYNET_REMOTEAPP_MEDIA_BACKPRESSURE_RESET_COMMAND' "$MEDIA_ADAPTATION_RUNNER" \
  'media adaptation host runner must require an explicit backpressure reset'
require 'trap reset_active_fixture EXIT' "$MEDIA_ADAPTATION_RUNNER" \
  'media adaptation host runner must reset active fixture state on every exit'
require 'aggregate-remoteapp-media-adaptation-evidence\.py' "$MEDIA_ADAPTATION_RUNNER" \
  'media adaptation host runner must use the canonical evidence aggregator'
require 'media_pipeline_support' "$MATRIX" \
  'matrix must record media pipeline support projection evidence'
require 'media_pipeline_support' "$PLAN" \
  'plan evidence audit must record media pipeline support projection evidence'
require 'Linux display is diagnostic-only' "$AUDIT" \
  'audit must record Linux display diagnostic-only support state'
require 'Linux and Windows rows expose an executable xcap/OpenH264 `baseline_ready`' "$AUDIT" \
  'audit must record executable but uncertified Windows/Linux capture state'
require 'Linux/Windows input injection has guarded executable baselines' "$AUDIT" \
  'audit must record guarded but uncertified Linux/Windows input state'
require 'governed Hub routing, cross-device ability visibility/invocation' "$AUDIT" \
  'audit must scope cross-device smoke to routing and synthetic media evidence'
require 'does not prove real' "$AUDIT" \
  'audit must reject cross-device smoke as real OS capture proof'
require 'Service owner multihost read-model conflict' "$AUDIT" \
  'audit must record the historical cross-device service owner projection diagnosis'
require 'service_owner_projection_is_fenced_per_host_device' "$AUDIT" \
  'audit must record the Service multihost projection regression evidence'
require 'docker info timed out after 3s' "$AUDIT" \
  'audit must record the latest live cross-device smoke environment failure'
require '20260823-rich-failure-check-70909' "$AUDIT" \
  'audit must record the latest Hub API readiness diagnostic artifact'
require '20260823-live-preflight-82429' "$AUDIT" \
  'audit must record the latest full product-flow preflight failure artifact'
require '20260823-hydrated-health-report-21626' "$AUDIT" \
  'audit must record the hydrated Hub API readiness health-failure artifact'
require '20260823-hydrated-health-report-21627' "$AUDIT" \
  'audit must record the hydrated full product-flow health-failure artifact'
require 'connection refused' "$AUDIT" \
  'audit must preserve the current Hub API health connection-refused blocker'
require 'START_FAILED_CREDENTIAL_VERIFY' "$AUDIT" \
  'audit must preserve the current credential-verification blocker'
require 'T06_VERIFY_CREDENTIAL' "$AUDIT" \
  'audit must preserve the credential-verification failure stage'
require 'hub_api_endpoint=null' "$AUDIT" \
  'audit must preserve the missing Hub API endpoint blocker'
require 'source-contract checker, unit test, local provider' "$AUDIT" \
  'audit must name weak evidence classes'
require 'benchmark, or SPEC statement is insufficient' "$AUDIT" \
  'audit must define authoritative product evidence strictly'
require 'RemoteApp interactive desktop product: incomplete' "$AUDIT" \
  'audit must preserve the current product status'
require 'remoteapp-product-readiness-matrix.json' "$AUDIT" \
  'audit must name the machine-readable product readiness matrix'
require 'host-remoteapp-session-timeout-e2e\.sh' "$AUDIT" \
  'audit must record the host session timeout E2E harness'
require 'host-remoteapp-session-cancel-e2e\.sh' "$AUDIT" \
  'audit must record the host session cancel E2E harness'
require 'host-remoteapp-permission-revoke-e2e\.sh' "$AUDIT" \
  'audit must record the host permission revoke E2E harness'
require 'real platform permission revoke' "$AUDIT" \
  'audit must distinguish permission revoke harness from completed real OS evidence'
require 'host-remoteapp-session-resume-e2e\.sh' "$AUDIT" \
  'audit must record the host session resume E2E harness'
require 'lease refresh' "$AUDIT" \
  'audit must describe session resume as lease refresh evidence'
require 'lease refresh survival' "$AUDIT" \
  'audit must distinguish lease survival from browser transport resume'
require 'Browser transport resume is a' "$AUDIT" \
  'audit must require real browser transport-resume evidence'
require 'old PeerConnection' "$AUDIT" \
  'audit must require retirement of the prior browser transport generation'
require 'newer daemon-issued transport epoch' "$AUDIT" \
  'audit must bind browser resume to Runtime-issued transport generations'
require 'remoteapp-crash-restart-recovery-e2e\.sh' "$AUDIT" \
  'audit must record the crash/restart recovery evidence verifier'
require 'session_not_found' "$AUDIT" \
  'audit must record current live crash/restart session loss evidence'
require 'RemoteDesktopRecoveryStore' "$AUDIT" \
  'audit must record the new RemoteApp recovery store contract'

require 'Full interactive RemoteApp product: incomplete' "$PLAN" \
  'plan evidence audit must keep the goal open'
require 'host-remoteapp-session-timeout-e2e\.sh' "$PLAN" \
  'plan evidence audit must record the host session timeout E2E harness'
require 'host-remoteapp-session-cancel-e2e\.sh' "$PLAN" \
  'plan evidence audit must record the host session cancel E2E harness'
require 'host-remoteapp-permission-revoke-e2e\.sh' "$PLAN" \
  'plan evidence audit must record the host permission revoke E2E harness'
require 'requires a live run with real platform permission revoke' "$PLAN" \
  'plan evidence audit must not claim permission revoke product completion from self-test'
require 'host-remoteapp-session-resume-e2e\.sh' "$PLAN" \
  'plan evidence audit must record the host session resume E2E harness'
require 'waits past the original lease' "$PLAN" \
  'plan evidence audit must record session resume original-lease survival evidence'
require 'Browser transport resume is now an independent product-gate domain' "$PLAN" \
  'plan evidence audit must separate browser transport recovery from lease refresh'
require 'newly connected PeerConnection' "$PLAN" \
  'plan evidence audit must require a replacement browser transport generation'
require '2026-08-26 live Browser run now passed this contract' "$PLAN" \
  'plan evidence audit must record the live browser transport-resume evidence'
require 'Crash/restart recovery E2E' "$PLAN" \
  'plan evidence audit must list missing live crash/restart recovery evidence'
require 'remoteapp-crash-restart-recovery-e2e\.sh' "$PLAN" \
  'plan evidence audit must record the crash/restart recovery verifier'
require 'session_not_found' "$PLAN" \
  'plan evidence audit must preserve live crash/restart session loss evidence'
require 'RemoteDesktopRecoveryStore' "$PLAN" \
  'plan evidence audit must record the recovery store contract'
require 'Cross-platform capture implementation/evidence using' "$PLAN" \
  'plan evidence audit must list missing Windows/Linux evidence'
require 'remoteapp-cross-platform-capture-e2e\.sh' "$PLAN" \
  'plan evidence audit must record the cross-platform capture verifier'
require 'check-remoteapp-main-crate-implementation-tests\.sh' "$PLAN" \
  'plan evidence audit must record the main-crate implementation test gate'
require 'production-vs-diagnostic target-subject' "$PLAN" \
  'plan evidence audit must record production-vs-diagnostic target-subject projection coverage'
require 'diagnostic_target_subjects' "$MATRIX" \
  'product readiness matrix must record diagnostic target subject projection evidence'
require 'platform_support' "$MATRIX" \
  'product readiness matrix must record platform support projection evidence'
require 'input_control_support' "$MATRIX" \
  'product readiness matrix must record input control support projection evidence'
require 'lease-refresh survival evidence only' "$MATRIX" \
  'product readiness matrix must not equate lease survival with browser transport resume'
require 'opt-in real browser transport-resume path' "$MATRIX" \
  'product readiness matrix must record the executable browser transport-resume contract'
require '2026-08-26 live Browser window restart-resume artifact' "$MATRIX" \
  'product readiness matrix must record the live browser transport-resume artifact'
require 'labels Windows/Linux exact-target capture baseline_ready rather than production_ready' "$MATRIX" \
  'product readiness matrix must record executable but uncertified Windows/Linux capture state'
require 'Windows/Linux capture or explicit product unsupported state' "$PLAN" \
  'plan evidence audit must preserve Windows/Linux capture or unsupported requirement'
require 'Real input injection E2E for pointer/keyboard using' "$PLAN" \
  'plan evidence audit must list missing live input injection evidence'
require 'remoteapp-input-injection-e2e\.sh' "$PLAN" \
  'plan evidence audit must record the input injection verifier'
require 'Audio/video media adaptation E2E' "$PLAN" \
  'plan evidence audit must list missing live audio/video media evidence'
require 'remoteapp-media-adaptation-e2e\.sh' "$PLAN" \
  'plan evidence audit must record the media adaptation verifier'
require 'Multi-window tracking E2E' "$PLAN" \
  'plan evidence audit must list missing live multi-window tracking evidence'
require 'remoteapp-multi-window-tracking-e2e\.sh' "$PLAN" \
  'plan evidence audit must record the multi-window tracking verifier'
require 'Frontend full lifecycle E2E' "$PLAN" \
  'plan evidence audit must list frontend full lifecycle E2E as missing'
require 'frontend-remoteapp-browser-lifecycle-e2e\.sh' "$PLAN" \
  'plan evidence audit must record the Browser/Tauri lifecycle verifier'
require 'submitted data-channel frame and daemon applied event target_focus_epoch' "$MATRIX" \
  'product readiness matrix must record frontend input focus-epoch evidence requirement'
require 'remoteapp-cross-device-product-smoke.sh' "$PLAN" \
  'plan evidence audit must record the cross-device smoke gate'
require 'remoteapp-product-completion-e2e.sh' "$PLAN" \
  'plan evidence audit must record the product-completion evidence gate'
require 'remoteapp-product-finalize.py' "$PLAN" \
  'plan evidence audit must record the independent completion finalizer'
require 'frontend product-flow, Browser/Tauri' "$PLAN" \
  'plan evidence audit must summarize product-completion required reports'
require 'stable `script` identity' "$PLAN" \
  'plan evidence audit must record product-completion report provenance validation'
require 'target_kind=both' "$PLAN" \
  'plan evidence audit must record both-target product-flow completion coverage'
require 'target-narrowed' "$PLAN" \
  'plan evidence audit must reject narrowed product-flow evidence as product completion'
require 'traceable `result.json` step artifacts' "$PLAN" \
  'plan evidence audit must record product-flow step artifact traceability'
require 'Historical local cross-device `--run` evidence' "$PLAN" \
  'plan evidence audit must classify the Service projection failure as historical evidence'
require 'accepted_count=0, expected_count=5' "$PLAN" \
  'plan evidence audit must preserve the historical service owner projection failure evidence'
require 'service_owner_projection_selects_live_host_from_multihost_rows' "$PLAN" \
  'plan evidence audit must record the live-host Service route regression evidence'
require 'docker info timed out after 3s' "$PLAN" \
  'plan evidence audit must preserve the latest structured cross-device environment failure'
require '20260823-rich-failure-check-70909' "$PLAN" \
  'plan evidence audit must record the latest Hub API readiness diagnostic artifact'
require '20260823-live-preflight-82429' "$PLAN" \
  'plan evidence audit must record the latest full product-flow preflight failure artifact'
require '20260823-hydrated-health-report-21626' "$PLAN" \
  'plan evidence audit must record the hydrated Hub API readiness health-failure artifact'
require '20260823-hydrated-health-report-21627' "$PLAN" \
  'plan evidence audit must record the hydrated full product-flow health-failure artifact'
require 'connection refused' "$PLAN" \
  'plan evidence audit must preserve the current Hub API health connection-refused blocker'
require 'START_FAILED_CREDENTIAL_VERIFY' "$PLAN" \
  'plan evidence audit must preserve the current credential-verification blocker'
require 'hub_api_endpoint=null' "$PLAN" \
  'plan evidence audit must preserve the missing Hub API endpoint blocker'
require 'real OS' "$PLAN" \
  'plan evidence audit must preserve real OS non-claims'
require 'NAT/STUN/TURN relay' "$PLAN" \
  'plan evidence audit must preserve cross-device non-claims'
require 'remoteapp-network-fallback-e2e\.sh' "$PLAN" \
  'plan evidence audit must record the network fallback verifier'

require 'docker-two-node-easyremote-cli-e2e.sh' "$CROSS_DEVICE_SMOKE" \
  'cross-device smoke must compose the two-node routing E2E'
require 'docker-media-bidi-e2e.sh' "$CROSS_DEVICE_SMOKE" \
  'cross-device smoke must compose the media/bidi E2E'
require 'write_report "skipped"' "$CROSS_DEVICE_SMOKE" \
  'cross-device smoke must not default to false pass evidence'
require 'service_owner_projection_failed' "$CROSS_DEVICE_SMOKE" \
  'cross-device smoke must retain diagnostics for legacy Service owner projection failures'
require '"source"' "$CROSS_DEVICE_SMOKE" \
  'cross-device smoke must report source provenance'
require '"runtime"' "$CROSS_DEVICE_SMOKE" \
  'cross-device smoke must report runtime image provenance'
require 'image_created' "$CROSS_DEVICE_SMOKE" \
  'cross-device smoke must report runtime image creation time'
require 'build_requested' "$CROSS_DEVICE_SMOKE" \
  'cross-device smoke must report whether runtime image rebuild was requested'
require 'product_complete_claim.*False|product_complete_claim.*false' "$CROSS_DEVICE_SMOKE" \
  'cross-device smoke must reject product completion claims'
require 'requires_distinct_devices' "$CROSS_DEVICE_SMOKE" \
  'cross-device smoke must require a distinct-device topology'
require 'observed_device_pairs' "$CROSS_DEVICE_SMOKE" \
  'cross-device smoke must report observed caller/provider device pairs'
require 'distinct_device_uras_observed' "$CROSS_DEVICE_SMOKE" \
  'cross-device smoke must report whether distinct device URAs were observed'
require 'local_provider_boundary_only' "$CROSS_DEVICE_SMOKE" \
  'cross-device smoke must expose local-provider-only evidence as insufficient'
require 'distinct device URAs were not observed' "$CROSS_DEVICE_SMOKE" \
  'cross-device smoke must fail when distinct device URAs are not observed'
require 'local_provider_boundary_only=true' "$CROSS_DEVICE_SMOKE" \
  'cross-device smoke must fail local-provider-only passed runs'
require 'does not prove real OS window/application capture' "$CROSS_DEVICE_SMOKE" \
  'cross-device smoke must preserve product non-claims'
require 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_FRONTEND_PRODUCT_FLOW_REPORT_JSON' "$PRODUCT_COMPLETION" \
  'product-completion gate must require frontend product-flow evidence'
require 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_BROWSER_LIFECYCLE_REPORT_JSON' "$PRODUCT_COMPLETION" \
  'product-completion gate must require Browser/Tauri lifecycle evidence'
require 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_BROWSER_TRANSPORT_RESUME_REPORT_JSON' "$PRODUCT_COMPLETION" \
  'product-completion gate must require real browser transport-resume evidence separately from lease survival'
require 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_SMOKE_REPORT_JSON' "$PRODUCT_COMPLETION" \
  'product-completion gate must require cross-device smoke evidence'
require 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_REMOTEAPP_REPORT_JSON' "$PRODUCT_COMPLETION" \
  'product-completion gate must require cross-device RemoteApp product evidence'
require 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_PLATFORM_CAPTURE_REPORT_JSON' "$PRODUCT_COMPLETION" \
  'product-completion gate must require cross-platform capture evidence'
require 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_INPUT_INJECTION_REPORT_JSON' "$PRODUCT_COMPLETION" \
  'product-completion gate must require input injection evidence'
require 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MEDIA_ADAPTATION_REPORT_JSON' "$PRODUCT_COMPLETION" \
  'product-completion gate must require media adaptation evidence'
require 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MULTI_WINDOW_TRACKING_REPORT_JSON' "$PRODUCT_COMPLETION" \
  'product-completion gate must require multi-window tracking evidence'
require 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_NETWORK_FALLBACK_REPORT_JSON' "$PRODUCT_COMPLETION" \
  'product-completion gate must require network fallback evidence'
require 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_TIMEOUT_WINDOW_REPORT_JSON' "$PRODUCT_COMPLETION" \
  'product-completion gate must require window session timeout evidence'
require 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_TIMEOUT_APPLICATION_REPORT_JSON' "$PRODUCT_COMPLETION" \
  'product-completion gate must require application session timeout evidence'
require 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_CANCEL_WINDOW_REPORT_JSON' "$PRODUCT_COMPLETION" \
  'product-completion gate must require window session cancel evidence'
require 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_CANCEL_APPLICATION_REPORT_JSON' "$PRODUCT_COMPLETION" \
  'product-completion gate must require application session cancel evidence'
require 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_PERMISSION_REVOKE_WINDOW_REPORT_JSON' "$PRODUCT_COMPLETION" \
  'product-completion gate must require window permission revoke evidence'
require 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_PERMISSION_REVOKE_APPLICATION_REPORT_JSON' "$PRODUCT_COMPLETION" \
  'product-completion gate must require application permission revoke evidence'
require 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_RESUME_WINDOW_REPORT_JSON' "$PRODUCT_COMPLETION" \
  'product-completion gate must require window session resume evidence'
require 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_RESUME_APPLICATION_REPORT_JSON' "$PRODUCT_COMPLETION" \
  'product-completion gate must require application session resume evidence'
require 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CRASH_RESTART_RECOVERY_REPORT_JSON' "$PRODUCT_COMPLETION" \
  'product-completion gate must require crash/restart recovery evidence'
require 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CAMPAIGN_BUNDLE_JSON' "$PRODUCT_COMPLETION" \
  'product-completion gate must require a signed live campaign bundle'
reject 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_ATTESTATION_TRUST_JSON' "$PRODUCT_COMPLETION" \
  'product-completion gate must not accept a caller-selected attestation trust bundle'
reject 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_REPLAY_LEDGER_DIR' "$PRODUCT_COMPLETION" \
  'product-completion gate must not accept a caller-selected replay ledger'
require '/etc/easynet/remoteapp-attestation-trust\.json' "$PRODUCT_COMPLETION" \
  'product-completion gate must use a system-provisioned Linux trust root'
require '/var/lib/easynet/remoteapp-campaign-replay' "$PRODUCT_COMPLETION" \
  'product-completion gate must use a system-provisioned Linux replay ledger'
require 'and campaign_verified' "$PRODUCT_COMPLETION" \
  'product-completion eligibility must be gated by verified signed campaign evidence'
require 'easynet\.remoteapp\.product-completion-candidate\.v1' "$PRODUCT_COMPLETION" \
  'the evidence aggregator must emit a non-claim completion candidate'
require 'product_complete_claim = False' "$PRODUCT_COMPLETION" \
  'the evidence aggregator must never mint a product-complete claim'
require 'COMPLETION_ROLE = "product_completion_authority"' "$PRODUCT_FINALIZER" \
  'the final report must require an independent product-completion authority'
require 'verify_dsse_envelope' "$PRODUCT_FINALIZER" \
  'the finalizer must cryptographically verify its completion decision'
require 'candidate_sha256' "$PRODUCT_FINALIZER" \
  'the completion decision must bind the exact candidate bytes'
require 'REQUIRED_PRODUCT_DOMAIN_IDS' "$PRODUCT_FINALIZER" \
  'completion authority must independently pin the complete product domain set'
require 'campaign verification domains do not match product checks' "$PRODUCT_FINALIZER" \
  'completion authority must reject candidate/campaign domain-set drift'
require 'prepare_completion_signing_material' "$PRODUCT_FINALIZER" \
  'completion authority must produce constrained DSSE signing material'
require 'assemble_completion_attestation' "$PRODUCT_FINALIZER" \
  'completion authority must verify external signatures before envelope assembly'
require 'dsse_pae' "$PRODUCT_FINALIZER" \
  'completion signing material must use canonical DSSE PAE'
require 'signature must contain exactly 64 bytes' "$PRODUCT_FINALIZER" \
  'completion envelope assembly must require a canonical Ed25519 signature'
require 'candidate_b64' "$PRODUCT_FINALIZER" \
  'the final report must carry the exact candidate bytes for standalone verification'
require 'verify_final_report' "$PRODUCT_FINALIZER" \
  'product-complete reports must expose a cryptographic verification path'
require 'verify_campaign_replay' "$PRODUCT_FINALIZER" \
  'final-report verification must bind the system replay record'
require 'product completion authority must have independent key and signer identity' "$PRODUCT_FINALIZER" \
  'completion authority custody must be independent from campaign and observers'
require 'reserve_campaign_replay' "$PRODUCT_FINALIZER" \
  'campaign replay state must be consumed only by the signed finalizer'
require 'allow_exact_existing=True' "$PRODUCT_FINALIZER" \
  'final report publication must recover after replay reservation without accepting a different decision'
require '"product_complete_claim": True' "$PRODUCT_FINALIZER" \
  'only the signed completion finalizer may mint the product-complete claim'
reject 'trust-bundle' "$PRODUCT_FINALIZER" \
  'the completion finalizer must not accept a caller-selected trust root'
reject 'replay-ledger' "$PRODUCT_FINALIZER" \
  'the completion finalizer must not accept a caller-selected replay ledger'
reject 'private-key|private_key' "$PRODUCT_FINALIZER" \
  'completion workflow must keep private signing keys outside the repository tool'
require 'CAMPAIGN_BUNDLE_SCHEMA = "easynet\.remoteapp\.campaign-bundle\.v2"' "$EVIDENCE_PROVENANCE" \
  'RemoteApp provenance verifier must pin the signed campaign bundle schema'
require 'RECEIPT_PROOF_SET_SCHEMA = "easynet\.remoteapp\.receipt-proof-set\.v2"' "$EVIDENCE_PROVENANCE" \
  'RemoteApp receipt proofs must bind the signed campaign challenge'
require 'derive-invocation-nonce' "$EVIDENCE_PROVENANCE" \
  'RemoteApp live runners must be able to derive campaign-bound invocation nonces'
require 'append-receipt-proof' "$EVIDENCE_PROVENANCE" \
  'RemoteApp live runners must be able to assemble verified receipt proof sets'
require 'EASYNET_REMOTEAPP_CAMPAIGN_RECEIPT_PROOF_SET_JSON' "$DECODED_FRAME_PROBE" \
  'the real decoded-frame runner must accept campaign proof output'
require 'append-receipt-proof' "$DECODED_FRAME_PROBE" \
  'the real decoded-frame runner must append its verified create-session receipt'
require '\("session_id", proof\.session_id\.as_bytes\(\)\)' "$RECEIPT_VERIFICATION" \
  'campaign invocation nonces must bind the RemoteApp session id'
require 'TRUST_SCHEMA = "easynet\.remoteapp\.attestation-trust\.v3"' "$EVIDENCE_PROVENANCE" \
  'RemoteApp product trust must expose rotation and revocation lifecycle'
require 'require_trusted_key_active' "$EVIDENCE_PROVENANCE" \
  'campaign and observer signatures must enforce trust-key lifecycle'
require 'campaign_invocation_nonce' "$PRODUCT_COMPLETION" \
  'product receipt verification must compare every challenge-derived nonce'
require 'remote_desktop\.focus_target' "$PRODUCT_COMPLETION" \
  'interactive product evidence must prove a successful focus invocation'
require 'def rotate' "$ATTESTATION_TRUST_ADMIN" \
  'RemoteApp product authority must support public-key rotation'
require 'def revoke' "$ATTESTATION_TRUST_ADMIN" \
  'RemoteApp product authority must support public-key revocation'
require 'def install' "$ATTESTATION_TRUST_ADMIN" \
  'RemoteApp product authority must have a fixed-path privileged installer'
require 'dsse_pae' "$EVIDENCE_PROVENANCE" \
  'RemoteApp provenance verifier must verify DSSE pre-authenticated payloads'
require 'openssl_verify_ed25519' "$EVIDENCE_PROVENANCE" \
  'RemoteApp provenance verifier must cryptographically verify Ed25519 signatures'
require 'reserve_campaign_replay' "$EVIDENCE_PROVENANCE" \
  'RemoteApp provenance verifier must atomically reject reused campaign ids'
require 'verify_attested_file' "$EVIDENCE_PROVENANCE" \
  'RemoteApp provenance verifier must recompute report and artifact digests'
require 'read_verified_json' "$EVIDENCE_PROVENANCE" \
  'RemoteApp semantic reads must remain bound to the recursive signed manifest'
require 'must not combine authority roles' "$EVIDENCE_PROVENANCE" \
  'campaign, observer, and completion authority custody must remain separated'
require 'verify_campaign_receipts' "$PRODUCT_COMPLETION" \
  'product-completion gate must run the Axon receipt verifier over every domain'
require 'receipt_verifier_sha256' "$PRODUCT_COMPLETION" \
  'the receipt verifier binary must be bound to the signed campaign build'
require 'verify_finalization_proof_set_with_resolver' "$RECEIPT_VERIFICATION" \
  'RemoteApp receipt proof verification must delegate to the Axon finalization verifier'
require 'verify_wire_finalization_checkpoints' "$RECEIPT_VERIFICATION" \
  'RemoteApp receipt verification must use canonical admission/terminal checkpoints'
require 'topology\.local_provider_boundary_only is not false' "$PRODUCT_COMPLETION" \
  'product-completion gate must reject local-provider-only cross-device reports'
require 'child verifier must not claim product completion' "$PRODUCT_COMPLETION" \
  'product-completion gate must reject child product-complete claims'
require 'requires_evidence_json' "$PRODUCT_COMPLETION" \
  'product-completion gate must require live evidence_json artifacts from domain verifiers'
require 'requires_frontend_flow_summary' "$PRODUCT_COMPLETION" \
  'product-completion gate must require frontend product-flow summaries'
require 'requires_transport_resume_summary' "$PRODUCT_COMPLETION" \
  'product-completion gate must require a real browser transport-resume summary'
require 'real_browser_transport_resume' "$PRODUCT_COMPLETION" \
  'product-completion gate must reject lease-refresh evidence as browser transport resume'
require 'transport_resume_summary.transport_epoch must exceed prior_transport_epoch' "$PRODUCT_COMPLETION" \
  'product-completion gate must require a newer daemon-issued transport generation'
require 'self-test accepted browser transport resume without a new PeerConnection' "$PRODUCT_COMPLETION" \
  'product-completion gate self-test must reject transport resume without PeerConnection replacement'
require 'frontend_flow_summary must be an object' "$PRODUCT_COMPLETION" \
  'product-completion gate must reject frontend product-flow reports without summaries'
require 'self-test accepted frontend product-flow report without summary' "$PRODUCT_COMPLETION" \
  'product-completion gate self-test must reject missing frontend product-flow summaries'
require 'requires_platforms_passed' "$PRODUCT_COMPLETION" \
  'product-completion gate must require cross-platform product evidence to pass rather than report unsupported'
require 'requires_cross_platform_capture_scenarios' "$PRODUCT_COMPLETION" \
  'product-completion gate must require cross-platform capture scenario summaries'
require 'cross-platform capture .* scenarios summary must be a non-empty list' "$PRODUCT_COMPLETION" \
  'product-completion gate must reject cross-platform capture reports without per-target summaries'
require 'self-test accepted cross-platform capture report without scenarios' "$PRODUCT_COMPLETION" \
  'product-completion gate self-test must reject missing cross-platform capture scenarios'
require 'unsupported_targets must be empty' "$PRODUCT_COMPLETION" \
  'product-completion gate must reject unsupported cross-platform capture targets'
require 'requires_input_injection_scenarios' "$PRODUCT_COMPLETION" \
  'product-completion gate must require input injection summaries'
require 'input injection .* input_summary must be an object' "$PRODUCT_COMPLETION" \
  'product-completion gate must reject input reports without per-platform summaries'
require 'self-test accepted input injection report without summaries' "$PRODUCT_COMPLETION" \
  'product-completion gate self-test must reject missing input injection summaries'
require "expected 'passed'" "$PRODUCT_COMPLETION" \
  'product-completion gate must reject unsupported input platforms'
require 'expected_target_kind' "$PRODUCT_COMPLETION" \
  'product-completion gate must require full frontend product-flow target coverage'
require 'target_kind is' "$PRODUCT_COMPLETION" \
  'product-completion gate must reject frontend product-flow reports that are not target_kind=both'
require 'product-flow subreport .* target_kind is' "$PRODUCT_COMPLETION" \
  'product-completion gate must reject swapped window/application host subreport target kinds'
require 'host-decoded-frame-window' "$PRODUCT_COMPLETION" \
  'product-completion gate must require window decoded-frame product-flow evidence'
require 'host-target-picker-freshness-application' "$PRODUCT_COMPLETION" \
  'product-completion gate must require a distinct application target-picker freshness artifact'
require 'window_target_picker_fresh' "$PRODUCT_COMPLETION" \
  'product-completion gate must require window target-picker freshness in the frontend flow summary'
require 'application_target_picker_fresh' "$PRODUCT_COMPLETION" \
  'product-completion gate must require application target-picker freshness in the frontend flow summary'
require 'host-decoded-frame-application' "$PRODUCT_COMPLETION" \
  'product-completion gate must require application decoded-frame product-flow evidence'
require 'host-view-only-input-window' "$PRODUCT_COMPLETION" \
  'product-completion gate must require window view-only input product-flow evidence'
require 'host-view-only-input-application' "$PRODUCT_COMPLETION" \
  'product-completion gate must require application view-only input product-flow evidence'
require 'product_flow_step_artifacts' "$PRODUCT_COMPLETION" \
  'product-completion gate must require product-flow step artifacts'
require 'product-flow step result_json path does not exist' "$PRODUCT_COMPLETION" \
  'product-completion gate must fail when a product-flow step result artifact is missing'
require 'product-flow subreport evidence_json path does not exist' "$PRODUCT_COMPLETION" \
  'product-completion gate must fail when a product-flow subreport evidence artifact is missing'
require 'evidence_json status is' "$PRODUCT_COMPLETION" \
  'product-completion gate must reject evidence artifacts whose own status is not passed'
require 'LIVE_EVIDENCE_ORIGIN = "live_runner"' "$PRODUCT_COMPLETION" \
  'product-completion gate must define live evidence provenance'
require 'CONTRACT_SELF_TEST_ORIGIN = "contract_self_test"' "$PRODUCT_COMPLETION" \
  'product-completion gate must define contract self-test provenance'
require 'self-test accepted contract_self_test report evidence_origin' "$PRODUCT_COMPLETION" \
  'product-completion gate must mutation-test report provenance rejection'
require 'self-test accepted missing evidence_json evidence_origin' "$PRODUCT_COMPLETION" \
  'product-completion gate must mutation-test referenced evidence provenance rejection'
require 'self-test accepted unknown product-flow step evidence_origin' "$PRODUCT_COMPLETION" \
  'product-completion gate must mutation-test nested step provenance rejection'
require 'A run-mode invocation never mints live provenance' "$EVIDENCE_PROVENANCE" \
  'evidence provenance boundary must forbid verifier-minted live evidence'
require 'observed != expected' "$EVIDENCE_PROVENANCE" \
  'evidence provenance boundary must fail closed on mismatched origins'
require 'only a passed domain report may project evidence_origin' "$EVIDENCE_PROVENANCE" \
  'evidence provenance boundary must project only verified passing reports'
require 'self-test accepted wrong product-flow host subreport script identity' "$PRODUCT_COMPLETION" \
  'product-completion gate self-test must reject wrong product-flow host subreport script identities'
require 'self-test accepted wrong product-flow host subreport target_kind' "$PRODUCT_COMPLETION" \
  'product-completion gate self-test must reject wrong product-flow host subreport target kinds'
require 'evidence_json path does not exist' "$PRODUCT_COMPLETION" \
  'product-completion gate must fail when a report evidence_json artifact is missing'
require 'required product-flow step' "$PRODUCT_COMPLETION" \
  'product-completion gate must require explicit passed frontend product-flow steps'
require 'topology\.observed_device_pairs must not be empty' "$PRODUCT_COMPLETION" \
  'product-completion gate must require observed cross-device caller/provider pairs'
require 'self-test accepted missing evidence_json artifact' "$PRODUCT_COMPLETION" \
  'product-completion gate self-test must reject missing evidence_json artifacts'
require 'self-test accepted wrong lifecycle target_kind' "$PRODUCT_COMPLETION" \
  'product-completion gate self-test must reject wrong lifecycle target kinds'
require 'requires_lifecycle_summary' "$PRODUCT_COMPLETION" \
  'product-completion gate must require lifecycle summary evidence'
require 'lifecycle_summary must be an object' "$PRODUCT_COMPLETION" \
  'product-completion gate must reject lifecycle reports without summaries'
require 'self-test accepted lifecycle report without summary' "$PRODUCT_COMPLETION" \
  'product-completion gate self-test must reject missing lifecycle summaries'
require 'self-test accepted missing frontend product-flow step' "$PRODUCT_COMPLETION" \
  'product-completion gate self-test must reject incomplete frontend product-flow reports'
require 'self-test accepted product-flow target_kind other than both' "$PRODUCT_COMPLETION" \
  'product-completion gate self-test must reject narrowed frontend product-flow target coverage'
require 'self-test accepted missing product-flow step result artifact' "$PRODUCT_COMPLETION" \
  'product-completion gate self-test must reject missing product-flow step result artifacts'
require 'self-test accepted missing product-flow subreport evidence artifact' "$PRODUCT_COMPLETION" \
  'product-completion gate self-test must reject missing product-flow subreport evidence artifacts'
require 'self-test accepted failed product-flow subreport evidence status' "$PRODUCT_COMPLETION" \
  'product-completion gate self-test must reject failed product-flow subreport evidence status'
require 'self-test accepted failed evidence_json status' "$PRODUCT_COMPLETION" \
  'product-completion gate self-test must reject failed report evidence status'
require 'self-test accepted missing observed cross-device pairs' "$PRODUCT_COMPLETION" \
  'product-completion gate self-test must reject missing observed cross-device pairs'
require 'requires_cross_device_remoteapp_scenarios' "$PRODUCT_COMPLETION" \
  'product-completion gate must require cross-device RemoteApp target summaries'
require 'cross-device RemoteApp target .* remoteapp_summary must be an object' "$PRODUCT_COMPLETION" \
  'product-completion gate must reject cross-device RemoteApp reports without per-target summaries'
require 'self-test accepted cross-device RemoteApp report without summaries' "$PRODUCT_COMPLETION" \
  'product-completion gate self-test must reject missing cross-device RemoteApp summaries'
require 'self-test accepted unsupported cross-platform capture as product completion' "$PRODUCT_COMPLETION" \
  'product-completion gate self-test must reject unsupported capture product-completion evidence'
require 'self-test accepted unsupported input injection as product completion' "$PRODUCT_COMPLETION" \
  'product-completion gate self-test must reject unsupported input product-completion evidence'
require 'expected_script' "$PRODUCT_COMPLETION" \
  'product-completion gate must pin expected report script identities'
require 'report script is' "$PRODUCT_COMPLETION" \
  'product-completion gate must reject wrong report script identities'
require 'tools/scripts/host-remoteapp-session-timeout-e2e.sh' "$PRODUCT_COMPLETION" \
  'product-completion gate must require host timeout report provenance'
require 'tools/scripts/remoteapp-network-fallback-e2e.sh' "$PRODUCT_COMPLETION" \
  'product-completion gate must require network fallback report provenance'
require 'and not contract_fixture_mode' "$PRODUCT_COMPLETION" \
  'product-completion candidate eligibility must reject contract fixtures'
require 'contract_fixture and cannot be accepted as live evidence' "$PRODUCT_COMPLETION" \
  'product-completion gate must reject synthetic contract fixtures in live check mode'
require 'self-test accepted contract_fixture as live product evidence' "$PRODUCT_COMPLETION" \
  'product-completion gate must mutation-test fixture laundering rejection'
require 'frontend_flow_summary' "$FRONTEND_PRODUCT_FLOW" \
  'frontend product-flow verifier must emit product journey summaries'
require 'hub_api_ready' "$FRONTEND_PRODUCT_FLOW" \
  'frontend product-flow verifier must summarize Hub API readiness'
require 'product_runtime_ready' "$FRONTEND_PRODUCT_FLOW" \
  'frontend product-flow verifier must summarize product runtime readiness'
require 'frontend_typechecked' "$FRONTEND_PRODUCT_FLOW" \
  'frontend product-flow verifier must summarize frontend typecheck coverage'
require 'ui_flow_exercised' "$FRONTEND_PRODUCT_FLOW" \
  'frontend product-flow verifier must summarize RemoteApp UI flow coverage'
require 'browser_lifecycle_verified' "$FRONTEND_PRODUCT_FLOW" \
  'frontend product-flow verifier must summarize Browser/Tauri lifecycle coverage'
require 'cross_device_distinct_devices' "$FRONTEND_PRODUCT_FLOW" \
  'frontend product-flow verifier must summarize distinct-device coverage'
require 'permission_subject_checked' "$FRONTEND_PRODUCT_FLOW" \
  'frontend product-flow verifier must summarize permission subject coverage'
require 'target_picker_fresh' "$FRONTEND_PRODUCT_FLOW" \
  'frontend product-flow verifier must summarize target-picker freshness'
require 'window_target_picker_fresh' "$FRONTEND_PRODUCT_FLOW" \
  'frontend product-flow verifier must separately summarize window target-picker freshness'
require 'application_target_picker_fresh' "$FRONTEND_PRODUCT_FLOW" \
  'frontend product-flow verifier must separately summarize application target-picker freshness'
require 'host-target-picker-freshness-application' "$FRONTEND_PRODUCT_FLOW" \
  'frontend product-flow verifier must run a distinct application target-picker freshness step'
require 'run_step host-target-picker-freshness-application "\$TARGET_FRESHNESS"' "$FRONTEND_PRODUCT_FLOW" \
  'frontend product-flow verifier must execute the distinct application target-picker freshness step'
require 'window_frame_rendered' "$FRONTEND_PRODUCT_FLOW" \
  'frontend product-flow verifier must summarize window render coverage'
require 'application_frame_rendered' "$FRONTEND_PRODUCT_FLOW" \
  'frontend product-flow verifier must summarize application render coverage'
require 'window_view_only_input_checked' "$FRONTEND_PRODUCT_FLOW" \
  'frontend product-flow verifier must summarize window input policy coverage'
require 'application_view_only_input_checked' "$FRONTEND_PRODUCT_FLOW" \
  'frontend product-flow verifier must summarize application input policy coverage'
require 'end_session_lifecycle_verified' "$FRONTEND_PRODUCT_FLOW" \
  'frontend product-flow verifier must summarize end-session lifecycle coverage'
require 'transport_resume_summary' "$FRONTEND_BROWSER_LIFECYCLE" \
  'browser lifecycle verifier must project verified transport-resume evidence for product aggregation'
require 'real_browser_transport_resume' "$FRONTEND_BROWSER_LIFECYCLE" \
  'browser lifecycle verifier must distinguish real transport resume from lease survival'
require 'self-test accepted resume without a new PeerConnection' "$FRONTEND_BROWSER_LIFECYCLE" \
  'browser lifecycle verifier self-test must reject fake reconnect evidence'
for resume_step in \
  transport_disconnected \
  session_preserved_for_reconnect \
  transport_reconnected \
  watch_events_reestablished \
  media_presented_after_resume \
  input_control_after_resume; do
  require "$resume_step" "$FRONTEND_BROWSER_LIFECYCLE" \
    "browser lifecycle verifier must require $resume_step evidence"
done
require 'real_remoteapp_cross_device_session' "$CROSS_DEVICE_REMOTEAPP" \
  'cross-device RemoteApp verifier must require real RemoteApp cross-device proof mode'
require 'remoteapp_summary' "$CROSS_DEVICE_REMOTEAPP" \
  'cross-device RemoteApp verifier must emit product aggregate summaries'
require 'remote_target_inventory_seen' "$CROSS_DEVICE_REMOTEAPP" \
  'cross-device RemoteApp verifier must require remote target inventory evidence'
require 'remote_desktop.create_session' "$CROSS_DEVICE_REMOTEAPP" \
  'cross-device RemoteApp verifier must require governed create_session ability evidence'
require 'remote_desktop.set_description' "$CROSS_DEVICE_REMOTEAPP" \
  'cross-device RemoteApp verifier must require production WebRTC signaling instead of diagnostic attach'
require 'diagnostic remote_desktop.attach cannot prove the production WebRTC path' "$CROSS_DEVICE_REMOTEAPP" \
  'cross-device RemoteApp verifier must reject diagnostic attach as product media evidence'
require 'selected_candidate_pair_id' "$CROSS_DEVICE_REMOTEAPP" \
  'cross-device RemoteApp verifier must require a selected WebRTC candidate pair'
require 'rendered_on_client_endpoint' "$CROSS_DEVICE_REMOTEAPP" \
  'cross-device RemoteApp verifier must require Browser-endpoint-rendered media evidence'
require 'client_endpoint_id' "$CROSS_DEVICE_REMOTEAPP" \
  'cross-device RemoteApp verifier must bind the Browser media endpoint without promoting it to a Device'
require 'terminal_receipt must be visible' "$CROSS_DEVICE_REMOTEAPP" \
  'cross-device RemoteApp verifier must require terminal receipt evidence'
require '"script": "tools/scripts/host-remoteapp-session-timeout-e2e.sh"' "$SESSION_TIMEOUT" \
  'session timeout report must expose stable script provenance'
require '"script": "tools/scripts/host-remoteapp-session-cancel-e2e.sh"' "$SESSION_CANCEL" \
  'session cancel report must expose stable script provenance'
require '"script": "tools/scripts/host-remoteapp-permission-revoke-e2e.sh"' "$PERMISSION_REVOKE" \
  'permission revoke report must expose stable script provenance'
require '"script": "tools/scripts/host-remoteapp-session-resume-e2e.sh"' "$SESSION_RESUME" \
  'session resume report must expose stable script provenance'
require '"script": "tools/scripts/host-remoteapp-permission-subject-e2e.sh"' "$PERMISSION_SUBJECT" \
  'permission-subject product-flow report must expose stable script provenance'
require '"script": "tools/scripts/host-remoteapp-target-picker-freshness-e2e.sh"' "$TARGET_FRESHNESS" \
  'target-picker freshness product-flow report must expose stable script provenance'
require 'window_id_plus_owner_pid' "$TARGET_FRESHNESS" \
  'target-picker freshness must bind window identity to native window id and owner process'
require 'application_identity_plus_owner_pid_and_window_set' "$TARGET_FRESHNESS" \
  'target-picker freshness must bind application identity to owner process and exact window set'
require 'resolved_window_ids' "$TARGET_FRESHNESS" \
  'target-picker freshness must require the application resolved-window set'
require 'front_to_back_surfaces' "$TARGET_FRESHNESS" \
  'target-picker freshness must require the application surface membership projection'
require 'display_scoped.*False|display_scoped.*false' "$TARGET_FRESHNESS" \
  'target-picker freshness must prove application selection remains process-scoped'
require 'not resource_ura and target_pid is None and hint' "$TARGET_SELECTOR" \
  'target selector must keep diagnostic labels out of authoritative Resource URA/PID selection'
require 'window_id' "$TARGET_SELECTOR" \
  'target selector must require native window identity'
for selector_consumer in \
  "$TARGET_FRESHNESS" \
  "$DECODED_FRAME_PROBE" \
  "$VIEW_ONLY_INPUT_SAFETY" \
  "$SESSION_TIMEOUT" \
  "$SESSION_CANCEL" \
  "$PERMISSION_REVOKE" \
  "$SESSION_RESUME"; do
  require 'remoteapp-select-live-target\.py' "$selector_consumer" \
    "RemoteApp product flow must use canonical live target selection: $selector_consumer"
done
require '"script": "tools/scripts/host-remoteapp-decoded-frame-e2e.sh"' "$DECODED_FRAME" \
  'decoded-frame product-flow report must expose stable script provenance'
require '"script": "tools/scripts/host-remoteapp-view-only-input-safety-e2e.sh"' "$VIEW_ONLY_INPUT_SAFETY" \
  'view-only input product-flow report must expose stable script provenance'
require 'real_cross_platform_capture_matrix' "$CAPTURE_MATRIX" \
  'cross-platform capture verifier must require real capture matrix proof mode'
require 'component_mock.*False|component_mock.*false' "$CAPTURE_MATRIX" \
  'cross-platform capture verifier must reject component mock evidence'
require 'real_backend_runtime.*True|real_backend_runtime.*true' "$CAPTURE_MATRIX" \
  'cross-platform capture verifier must require real backend/runtime evidence'
require 'macos must pass display/window/application capture' "$CAPTURE_MATRIX" \
  'cross-platform capture verifier must require macOS live capture pass'
require 'explicit_product_unsupported' "$CAPTURE_MATRIX" \
  'cross-platform capture verifier must allow only explicit product unsupported state'
require 'show_unsupported' "$CAPTURE_MATRIX" \
  'cross-platform capture verifier must require visible unsupported UX state'
require 'first_display_capture_started' "$CAPTURE_MATRIX" \
  'cross-platform capture verifier must inspect first-display fallback evidence'
require 'display_fallback_used' "$CAPTURE_MATRIX" \
  'cross-platform capture verifier must reject display fallback for scoped targets'
require 'remote_desktop\.create_session' "$CAPTURE_MATRIX" \
  'cross-platform capture verifier must inspect create_session evidence'
require 'remote_desktop\.attach' "$CAPTURE_MATRIX" \
  'cross-platform capture verifier must inspect attach evidence'
require 'remote_desktop\.watch_events' "$CAPTURE_MATRIX" \
  'cross-platform capture verifier must inspect watch_events evidence'
require 'remote_desktop\.end_session' "$CAPTURE_MATRIX" \
  'cross-platform capture verifier must inspect end_session evidence'
require 'frames_rendered' "$CAPTURE_MATRIX" \
  'cross-platform capture verifier must inspect rendered frame evidence'
require 'target_identity' "$CAPTURE_MATRIX" \
  'cross-platform capture verifier must require selected target identity evidence'
require 'target_identity\.frame_source_id must be recorded' "$CAPTURE_MATRIX" \
  'cross-platform capture verifier must require target frame source evidence'
require 'target_identity\.geometry_revision must be positive' "$CAPTURE_MATRIX" \
  'cross-platform capture verifier must require target geometry revision evidence'
require 'rendered_frame_probe' "$CAPTURE_MATRIX" \
  'cross-platform capture verifier must require decoded-frame probe evidence'
require 'rendered_frame_probe.probe_source must be decoded_frame' "$CAPTURE_MATRIX" \
  'cross-platform capture verifier must require decoded-frame probe source'
require 'rendered_frame_probe frame_source_id must match target_identity' "$CAPTURE_MATRIX" \
  'cross-platform capture verifier must bind rendered frame source to target identity'
require 'rendered_frame_probe geometry_revision must match target_identity' "$CAPTURE_MATRIX" \
  'cross-platform capture verifier must bind rendered frame geometry to target identity'
require 'selected_sentinel_rendered' "$CAPTURE_MATRIX" \
  'cross-platform capture verifier must require selected sentinel render evidence'
require 'selected_sentinel_hash' "$CAPTURE_MATRIX" \
  'cross-platform capture verifier must require selected sentinel hash evidence'
require 'unrelated_sentinel_rendered' "$CAPTURE_MATRIX" \
  'cross-platform capture verifier must reject unrelated sentinel leakage'
require 'rendered_frame_probe unrelated_sentinel_rendered must be false' "$CAPTURE_MATRIX" \
  'cross-platform capture verifier must reject unrelated sentinel leakage in decoded-frame probe'
require 'terminal_receipt' "$CAPTURE_MATRIX" \
  'cross-platform capture verifier must inspect terminal receipt evidence'
require 'product_complete_claim.*False|product_complete_claim.*false' "$CAPTURE_MATRIX" \
  'cross-platform capture verifier must reject product completion claims'
require 'real_input_injection_matrix' "$INPUT_INJECTION" \
  'input injection verifier must require real input injection proof mode'
require 'component_mock.*False|component_mock.*false' "$INPUT_INJECTION" \
  'input injection verifier must reject component mock evidence'
require 'real_backend_runtime.*True|real_backend_runtime.*true' "$INPUT_INJECTION" \
  'input injection verifier must require real backend/runtime evidence'
require 'macos must pass pointer/keyboard input injection' "$INPUT_INJECTION" \
  'input injection verifier must require macOS live input pass'
require 'input_control' "$INPUT_INJECTION" \
  'input injection verifier must require input-control consent'
require 'display_global' "$INPUT_INJECTION" \
  'input injection verifier must require display_global input scope'
require 'target_local' "$INPUT_INJECTION" \
  'input injection verifier must require target_local scope for window/application targets'
require 'target_guard_validation' "$INPUT_INJECTION" \
  'input injection verifier must require per-event target guard evidence'
require 'fresh target snapshot and validation must precede OS apply' "$INPUT_INJECTION" \
  'input injection verifier must order fresh target validation before each OS event'
require 'focus_validated' "$INPUT_INJECTION" \
  'input injection verifier must require focus validation'
require 'coordinate_mapping_validated' "$INPUT_INJECTION" \
  'input injection verifier must require coordinate mapping validation'
require 'target_geometry_revision' "$INPUT_INJECTION" \
  'input injection verifier must inspect target geometry revision'
require 'target_focus_epoch must be positive' "$INPUT_INJECTION" \
  'input injection verifier must require focused target epoch evidence'
require 'target_focus_epoch: u64' "$ROOT/plugins/remote-desktop/src/target_tracking.rs" \
  'RemoteApp target tracker snapshot must own a daemon-side target focus epoch'
require 'target_focus_epoch_from_snapshot' "$INPUT" \
  'RemoteApp input policy must project target focus epoch from the current target snapshot'
require 'target_focus_epoch_reject_reason' "$INPUT" \
  'RemoteApp input execution must reject stale target focus epochs before OS injection'
require '"stale_target_focus_epoch"' "$INPUT" \
  'RemoteApp input execution must expose a stable stale focus epoch rejection reason'
require 'input_rejects_stale_target_focus_epoch_before_os_injection' "$INPUT" \
  'RemoteApp input tests must prove stale target focus epoch rejection happens before OS injection'
require 'target_focus_epoch"\.to_string\(\)' "$INPUT" \
  'RemoteApp applied input event payloads must preserve accepted target focus epoch evidence'
require 'INPUT_FRAME_APPLIED' "$INPUT_INJECTION" \
  'input injection verifier must inspect input-applied events'
require 'client_sequence' "$INPUT_INJECTION" \
  'input injection verifier must preserve client_sequence evidence'
require 'input_event_id must be recorded' "$INPUT_INJECTION" \
  'input injection verifier must require stable input event identity'
require 'rdinp1_<32 lowercase hex>' "$INPUT_INJECTION" \
  'input injection verifier must require daemon-shaped applied input event identity'
require 'transport_epoch must be positive' "$INPUT_INJECTION" \
  'input injection verifier must bind applied input to a transport epoch'
require 'accepted_count must be positive' "$INPUT_INJECTION" \
  'input injection verifier must bind applied input to daemon accepted-count ordering'
require 'client_sent_at_ms' "$INPUT_INJECTION" \
  'input injection verifier must preserve client_sent_at_ms evidence'
require 'host_received_at_ms' "$INPUT_INJECTION" \
  'input injection verifier must preserve host receive timing evidence'
require 'input_results client_sequence must be strictly increasing' "$INPUT_INJECTION" \
  'input injection verifier must reject non-monotonic applied input sequences'
require 'rejected_input_results' "$INPUT_INJECTION" \
  'input injection verifier must inspect rejected input evidence'
require 'stale_client_sequence rejection must be observed' "$INPUT_INJECTION" \
  'input injection verifier must require stale sequence rejection evidence'
require 'stale rejected input must not be host-applied' "$INPUT_INJECTION" \
  'input injection verifier must prove stale rejected input is not applied'
require 'struct InputSequenceGate' "$INPUT" \
  'RemoteApp daemon input execution path must have a per-channel client sequence gate'
require 'struct InputAppliedDiagnosticGate' "$INPUT" \
  'RemoteApp daemon input execution path must have bounded applied-input diagnostics'
require 'input_applied_diagnostic_gate_emits_first_success_for_each_kind' "$INPUT" \
  'RemoteApp input tests must prove first successful pointer and keyboard frames each emit applied evidence'
require 'sequence_gate\.reject_reason\(client_sequence\)' "$INPUT" \
  'RemoteApp daemon input data-channel loop must reject stale client sequences before input execution'
require '"stale_client_sequence"' "$INPUT" \
  'RemoteApp daemon input sequence gate must expose a stable stale sequence rejection reason'
require 'input_sequence_gate_rejects_replayed_or_out_of_order_frames' "$INPUT" \
  'RemoteApp daemon input tests must prove replayed or out-of-order client sequences are rejected'
require 'struct InputFrameTiming' "$INPUT" \
  'RemoteApp daemon input execution path must have a typed host timing projection'
require 'host_applied_at_ms' "$INPUT" \
  'RemoteApp daemon applied input events must expose host apply timestamps'
require 'latency_ms' "$INPUT" \
  'RemoteApp daemon input execution events must expose bounded latency telemetry'
require 'input_runtime_permission_denied\(reason\)' "$INPUT" \
  'RemoteApp daemon input execution path must detect runtime permission denial'
require 'mark_input_permission_blocked' "$INPUT" \
  'RemoteApp daemon input execution path must project runtime permission denial to the session aggregate'
require 'mark_input_frame_applied' "$INPUT" \
  'RemoteApp daemon input execution path must let successful host input application clear runtime permission blockers'
require 'input_runtime_block_reason' "$SESSION" \
  'RemoteApp session aggregate must retain runtime input permission block reason'
require 'session\.input_runtime_block_reason\(\)' "$SESSION_VIEW" \
  'RemoteApp show_session projection must expose session-local runtime input blockers through input_readiness'
require 'session_view_projects_session_local_runtime_input_blocker' "$SESSION_VIEW" \
  'RemoteApp session view tests must prove runtime input blockers survive show_session projection'
require 'INPUT_PERMISSION_BLOCKED' "$SESSION_EVENTS" \
  'RemoteApp session events must expose runtime input permission blocks'
require 'input_permission_block_projects_request_permission_recovery' "$SESSION_EVENTS" \
  'RemoteApp session event tests must prove runtime input permission blocks project request-permission recovery'
require 'INPUT_PERMISSION_RESTORED' "$SESSION_EVENTS" \
  'RemoteApp session events must expose runtime input permission restore'
require 'input_permission_restore_projects_resolved_recovery' "$SESSION_EVENTS" \
  'RemoteApp session event tests must prove runtime input permission restore projects resolved recovery'
require 'input_channel_diagnostic_projects_target_binding_context' "$SESSION_EVENTS" \
  'RemoteApp input-channel event projections must carry selected target binding evidence'
require 'blocked\["payload"\]\["target_binding"\]\["binding_id"\]' "$SESSION" \
  'RemoteApp input permission events must preserve selected target binding in the event log payload'
require 'restored\["payload"\]\["target_geometry_revision"\]' "$SESSION" \
  'RemoteApp input permission restore events must preserve target geometry revision'
require 'INPUT_PERMISSION_BLOCKED' "$EVENT_LOG" \
  'RemoteApp event log must classify runtime input permission blocks as input events'
require 'INPUT_PERMISSION_RESTORED' "$EVENT_LOG" \
  'RemoteApp event log must classify runtime input permission restores as input events'
require 'runtime_input_permission_block_deactivates_input_without_failing_media' "$SESSION" \
  'RemoteApp session tests must prove runtime input permission block does not fail media'
require 'latency_ms must be within threshold' "$INPUT_INJECTION" \
  'input injection verifier must reject high latency'
require 'observed_effect' "$INPUT_INJECTION" \
  'input injection verifier must require observed OS input effect'
require 'os_effect_probe_source' "$INPUT_INJECTION" \
  'input injection verifier must require platform OS-effect observer evidence'
require 'os_effect observer must be independent from injector' "$INPUT_INJECTION" \
  'input injection verifier must require independent OS-effect observer evidence'
require 'os_effect input_event_id must bind input_event_id' "$INPUT_INJECTION" \
  'input injection verifier must bind OS effect to the applied input event'
require 'os_effect observed_at_ms must be after host_applied_at_ms' "$INPUT_INJECTION" \
  'input injection verifier must require OS effect after host application'
require 'os_effect target_geometry_revision must match platform scenario' "$INPUT_INJECTION" \
  'input injection verifier must bind OS effect to target geometry revision'
require 'os_effect target_focus_epoch must match platform scenario' "$INPUT_INJECTION" \
  'input injection verifier must bind OS effect to target focus epoch'
require 'pointer OS effect must be observed within tolerance' "$INPUT_INJECTION" \
  'input injection verifier must require bounded pointer OS effect evidence'
require 'keyboard OS effect must bind focused Resource URA' "$INPUT_INJECTION" \
  'input injection verifier must require keyboard focus/resource binding'
require 'terminal_receipt' "$INPUT_INJECTION" \
  'input injection verifier must inspect terminal receipt evidence'
require 'product_complete_claim.*False|product_complete_claim.*false' "$INPUT_INJECTION" \
  'input injection verifier must reject product completion claims'
require 'EASYNET_REMOTEAPP_INPUT_PROOF=1' "$TARGET_INPUT_RUNNER" \
  'target-local input runner must enable production WebRTC input proof mode'
require 'remote_desktop\.set_description' "$TARGET_INPUT_RUNNER" \
  'target-local input runner must record the actual WebRTC signaling ability'
require 'remote_desktop\.watch_events' "$TARGET_INPUT_RUNNER" \
  'target-local input runner must observe session events through the public watch ability'
require 'remote_desktop\.end_session' "$TARGET_INPUT_RUNNER" \
  'target-local input runner must close through the public end-session ability'
require 'macos_appkit_target_observer' "$TARGET_INPUT_RUNNER" \
  'target-local input runner must bind independent AppKit target observations'
require 'unrelated AppKit target received RemoteApp input' "$TARGET_INPUT_RUNNER" \
  'target-local input runner must fail on unrelated target leakage'
require 'real_media_adaptation_matrix' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must require real media adaptation proof mode'
require 'expected_origin = "contract_self_test" if mode == "self-test" else "live_runner"' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must reject non-live evidence in run mode'
require 'evidence_origin.*contract_self_test' "$MEDIA_ADAPTATION" \
  'media adaptation verifier self-test must identify contract-only evidence'
require 'component_mock.*False|component_mock.*false' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must reject component mock evidence'
require 'real_backend_runtime.*True|real_backend_runtime.*true' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must require real backend/runtime evidence'
require 'baseline' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must require baseline scenario evidence'
require 'degraded_network' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must require degraded-network evidence'
require 'backpressure' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must require backpressure evidence'
require 'codec_negotiated' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must inspect codec negotiation'
require 'media_pipeline_id' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must bind comparable scenarios to one media pipeline'
require 'MEDIA_PIPELINE_STATS' "$SESSION_EVENTS" \
  'RemoteApp session events must expose target-bound media pipeline stats'
require 'media_pipeline_stats_projects_target_binding_context' "$SESSION_EVENTS" \
  'RemoteApp media pipeline stats projection must carry selected target binding evidence'
require 'latest\["payload"\]\["target_binding"\]\["subject_ura"\]' "$SESSION" \
  'RemoteApp media pipeline stats event-log rows must preserve selected Resource evidence'
require 'render_probe' "$REMOTE_DESKTOP_SCHEMA" \
  'RemoteApp report_client_state schema must accept render probe evidence'
require 'render_probe' "$REPORT_CLIENT_STATE_HANDLER" \
  'RemoteApp report_client_state handler must normalize render probe evidence'
for field in selected_resource_ura session_id transport_epoch binding_id binding_epoch media_source_epoch media_pipeline_id video_codec video_transport observed_at_ms decoded_video_frames frame_width frame_height; do
  require "$field" "$REPORT_CLIENT_STATE_DESCRIPTOR" \
    "RemoteApp packaged render-probe contract must declare required field $field"
  require "$field" "$REMOTE_DESKTOP_SCHEMA" \
    "RemoteApp NativeStatic render-probe contract must declare required field $field"
done
require 'authored_descriptor_and_runtime_schema_are_identical' "$REMOTE_DESKTOP_SCHEMA" \
  'RemoteApp packaged and NativeStatic report_client_state schemas must be parity-tested'
require 'struct ClientRenderEvidence' "$SESSION_TRANSPORT_STATE" \
  'RemoteApp render evidence must be typed and transport-generation scoped'
require 'client_decode_ready\(\)' "$SESSION" \
  'RemoteApp production readiness must be owned by the session aggregate exact decode predicate'
require 'client_decode_evidence_not_ready' "$SESSION_VIEW" \
  'RemoteApp product readiness must fail closed when presenting lacks bound decode evidence'
require 'struct HostAudioProbeCoordinator' "$HOST_AUDIO_CAPABILITY" \
  'RemoteApp host-audio probing must use a fixed-state plugin-owned coordinator'
require 'mpsc::sync_channel\(1\)' "$HOST_AUDIO_CAPABILITY" \
  'RemoteApp host-audio coordinator must use a capacity-one wake channel'
reject 'mpsc::channel\(\)' "$HOST_AUDIO_CAPABILITY" \
  'RemoteApp host-audio coordinator must not use an unbounded command channel'
require 'decoded_video_frames' "$REPORT_CLIENT_STATE_HANDLER" \
  'RemoteApp report_client_state handler must preserve decoded video frame evidence'
require 'video_payload_hash' "$REPORT_CLIENT_STATE_HANDLER" \
  'RemoteApp report_client_state handler must preserve render-probe payload fingerprints when provided'
require 'latest\["payload"\]\["stats"\]\["render_probe"\]' "$SESSION" \
  'RemoteApp media pipeline stats replay must preserve render probe evidence'
require 'remoteapp_media_pipeline_stats_v1' "$REMOTE_DESKTOP_MEDIA" \
  'RemoteApp media module must define the product media-pipeline stats contract'
require 'MEDIA_PIPELINE_STATS_CONTRACT' "$HOSTED_WEBRTC_MEDIA" \
  'hosted WebRTC bridge must emit the shared product media-pipeline stats contract'
require 'selected_resource_ura' "$HOSTED_WEBRTC_MEDIA" \
  'hosted WebRTC media stats must bind the selected Resource URA'
require '"media_pipeline_id": self\.media_pipeline_id' "$HOSTED_WEBRTC_MEDIA" \
  'hosted WebRTC media stats must bind the stable product media pipeline id'
require 'media_source_epoch' "$HOSTED_WEBRTC_MEDIA" \
  'hosted WebRTC media stats must bind the media source epoch'
require 'payload_content_type' "$HOSTED_WEBRTC_MEDIA" \
  'hosted WebRTC media stats must expose negotiated payload content type'
require 'measured_fps' "$HOSTED_WEBRTC_MEDIA" \
  'hosted WebRTC media stats must expose measured FPS'
require 'observed_bitrate_kbps' "$HOSTED_WEBRTC_MEDIA" \
  'hosted WebRTC media stats must expose observed bitrate'
require 'latency_stats' "$HOSTED_WEBRTC_MEDIA" \
  'hosted WebRTC media stats must expose bounded encode-to-WebRTC latency'
require 'requested_fps' "$HOSTED_WEBRTC_MEDIA" \
  'hosted WebRTC media stats must preserve requested FPS separately from effective FPS'
require 'effective_fps' "$HOSTED_WEBRTC_MEDIA" \
  'hosted WebRTC media stats must expose the applied media-host FPS'
require 'latest_frame_bounded_gop' "$HOSTED_WEBRTC_MEDIA" \
  'hosted WebRTC media stats must expose the bounded GOP-safe frame-drop policy'
require 'fn adaptation_events' "$HOSTED_WEBRTC_MEDIA" \
  'hosted WebRTC media bridge must bind adaptation events to session and pipeline context'
require 'backpressure_detected' "$HOSTED_WEBRTC_MEDIA" \
  'hosted WebRTC media bridge must project authenticated receiver pressure'
require 'audio_media_observed' "$HOSTED_WEBRTC_MEDIA" \
  'hosted WebRTC media stats must distinguish pipeline readiness from observed audio media'
require 'BoundedPendingWrites' "$WEBRTC_ENCODED_AUDIO" \
  'WebRTC audio writes must report through fixed-size shared state'
require 'ENCODED_AUDIO_QUEUE_DEPTH' "$WEBRTC_ENCODED_AUDIO" \
  'hosted WebRTC audio writer must hard-bound pending encoded packets'
require 'audio_transport_write_isolated' "$HOSTED_WEBRTC_MEDIA" \
  'hosted WebRTC media stats must expose audio transport-write isolation'
require 'bounded_queue_drop_oldest_audio_packet' "$HOSTED_WEBRTC_MEDIA" \
  'hosted WebRTC media stats must expose the stale-audio drop policy'
require 'struct OpusPacketizer' "$MEDIA_HOST_MAC_AUDIO" \
  'hosted macOS audio path must encode bounded PCM frames as Opus'
require 'struct HostedAudioTransport' "$HOSTED_WEBRTC_MEDIA" \
  'hosted WebRTC bridge must route media-host Opus through an independent writer'
require 'SCStreamOutputType::Audio' "$MEDIA_HOST_MAC" \
  'hosted ScreenCaptureKit adapter must register system-audio output'
require 'pub const SHARED_SLOT_NOTIFICATION_BYTES: usize = 56;' "$SHARED_MEDIA_LANE" \
  'media-host notifications must remain fixed tickets without codec payloads'
require 'Bytes::from_owner\(lease\)' "$ROOT/plugins/remote-desktop/src/native_host_process.rs" \
  'daemon media ingress must preserve mapped payload ownership into WebRTC Bytes'
require 'MIME_TYPE_OPUS' "$WEBRTC_ENDPOINT" \
  'WebRTC endpoint must register the Opus codec on the shared peer connection'
require 'scenario_started_at_ms must be recorded' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must require scenario start timestamp evidence'
require 'impairment_applied_at_ms must be after scenario_started_at_ms' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must require impairment timing evidence'
require 'selected_resource_ura must bind selected Resource URA' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must bind adaptation events to selected Resource URA'
require 'session_id must bind session_id' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must bind adaptation events to the session'
require 'media_pipeline_id must bind media_pipeline_id' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must bind adaptation events to the media pipeline'
require 'at_ms must be after impairment_applied_at_ms' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must require adaptation events after impairment'
require 'payload_content_type' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must preserve payload content type'
require 'requested_fps' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must inspect requested FPS'
require 'effective_fps' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must inspect effective FPS'
require 'measured_fps' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must inspect measured FPS'
require 'target_bitrate_kbps' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must inspect target bitrate'
require 'observed_bitrate_kbps' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must inspect observed bitrate'
require 'bitrate_downshift' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must require bitrate downshift evidence'
require 'degraded_network target_bitrate_kbps must be lower than baseline' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must require degraded target bitrate downshift'
require 'degraded_network observed_bitrate_kbps must be lower than baseline' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must require degraded observed bitrate downshift'
require 'degraded_network must reduce effective_fps or drop frames versus baseline' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must require degraded FPS/drop delta'
require 'frames_rendered_after_adaptation_at_ms must be after adaptation events' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must require rendered media after adaptation events'
require 'render_probe' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must require decoded render probe evidence'
require 'render_probe evidence must be present' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must require decoded render probe evidence'
require 'render_probe\.probe_source must be decoded_media_payload' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must require decoded media payload probe source'
require 'render_probe selected_resource_ura must bind selected Resource URA' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must bind render probe to selected Resource URA'
require 'render_probe session_id must bind session_id' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must bind render probe to session'
require 'render_probe media_pipeline_id must bind media_pipeline_id' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must bind render probe to media pipeline'
require 'render_probe video_codec must match negotiated video codec' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must bind render probe to negotiated video codec'
require 'render_probe video_transport must match negotiated video transport' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must bind render probe to negotiated video transport'
require 'render_probe audio_codec must match negotiated audio codec' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must bind render probe to negotiated audio codec'
require 'render_probe decoded_video_frames must be positive' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must require decoded video frame evidence'
require 'render_probe decoded audio packets or samples must be positive' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must require decoded audio evidence'
require 'render_probe video_payload_hash must be recorded' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must require video payload fingerprint evidence'
require 'render_probe audio_payload_hash must be recorded' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must require audio payload fingerprint evidence'
require 'render_probe observed_at_ms must be after adaptation events' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must order render probe after adaptation events'
require 'backpressure_detected' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must require backpressure detection evidence'
require 'frames_dropped' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must require frame drop evidence'
require 'backpressure frames_dropped must exceed baseline' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must require backpressure drop delta'
require 'selected_resource_ura must match across media scenarios' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must compare the same selected resource across scenarios'
require 'media_pipeline_id must match across media scenarios' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must compare one media pipeline across scenarios'
require 'video.codec must match across media scenarios' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must compare one video codec across scenarios'
require 'video.transport must match across media scenarios' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must compare one video transport across scenarios'
require 'allowed_transports = \{"webrtc", "native_webrtc"\}' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must reject generic raw-stream ABI as RemoteApp media proof'
reject 'raw_stream_v8' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must not treat generic raw-stream ABI as RemoteApp WebRTC proof'
require 'audio.codec must match across media scenarios' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must compare one audio codec across scenarios'
require 'audio.status must be passed' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must require live host audio evidence'
require 'host audio unsupported state is not product media evidence' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must reject host-audio unsupported evidence'
require 'queue.observed_max_depth must not exceed max_depth' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must reject unbounded queue evidence'
require 'audio transport writes must be isolated from the media control loop' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must require isolated audio transport writes'
require 'audio.queue.observed_max_depth must not exceed max_depth' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must reject unbounded audio queue evidence'
require 'audio.drop_policy must preserve the freshest bounded audio' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must require stale-audio drop semantics'
require 'audio sender backpressure drops must equal stale drops plus sender errors' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must require internally consistent audio drop counters'
require 'backpressure audio sender drops must exceed baseline' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must require observed audio backpressure behavior'
require 'remote_desktop\.create_session' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must inspect create_session evidence'
require 'remote_desktop\.set_description' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must inspect production WebRTC signaling evidence'
require 'remote_desktop\.watch_events' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must inspect watch_events evidence'
require 'remote_desktop\.end_session' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must inspect end_session evidence'
require 'terminal_receipt' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must inspect terminal receipt evidence'
require 'product_complete_claim.*False|product_complete_claim.*false' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must reject product completion claims'
require 'real_multi_window_tracking_matrix' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must require real tracking proof mode'
require 'component_mock.*False|component_mock.*false' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must reject component mock evidence'
require 'real_backend_runtime.*True|real_backend_runtime.*true' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must require real backend/runtime evidence'
require 'independent_window_streams' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must require independent stream scenario'
require 'geometry_churn' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must require geometry churn scenario'
require 'application_window_set_churn' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must require application window-set churn scenario'
require 'target_loss_rebind' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must require target loss/rebind scenario'
require 'multi_display_application' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must require multi-display application scenario'
require 'frames_interleaved must be false' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must reject interleaved streams'
require 'selected_sentinel_rendered' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must require selected target sentinel rendering'
require 'foreign_sentinel_rendered' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must reject foreign target sentinel leakage'
require 'sentinel_owner_resource_ura' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must bind rendered sentinel to selected Resource URA'
require 'cross_stream_sentinel_leakage' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must reject cross-stream sentinel leakage'
require 'rendered_frame_probe must be present' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must require decoded per-stream frame probe evidence'
require 'rendered_frame_probe\.probe_source must be decoded_frame' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must require decoded-frame stream probe source'
require 'rendered_frame_probe selected_resource_ura must bind selected stream' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must bind frame probe to selected stream Resource URA'
require 'rendered_frame_probe session_id must bind stream session' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must bind frame probe to stream session'
require 'rendered_frame_probe stream_id must bind stream' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must bind frame probe to stream id'
require 'rendered_frame_probe frame_source_id must bind stream frame source' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must bind frame probe to stream frame source'
require 'rendered_frame_probe media_source_epoch must bind stream' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must bind frame probe to media source epoch'
require 'rendered_frame_probe selected_sentinel_hash must be recorded' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must require per-stream sentinel hash evidence'
require 'rendered_frame_probe foreign_sentinel_rendered must be false' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must reject foreign sentinel leakage in decoded stream probe'
require 'TARGET_MOVED' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must inspect target move events'
require 'TARGET_RESIZED' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must inspect target resize events'
require 'PENDING_MEDIA_REBIND' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must inspect pending media rebind events'
require 'TARGET_REBOUND' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must inspect target rebound events'
require 'TARGET_LOST' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must inspect target loss events'
require 'TARGET_REBIND_FAILED' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must inspect target rebind failure events'
require 'first_display_capture_started' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must inspect first-display fallback evidence'
require 'display_fallback_used' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must reject display fallback for application churn'
require 'committed_window_set_sentinels_rendered_after_rebind' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must require committed window-set sentinel rendering after rebind'
require 'uncommitted_same_app_sentinel_rendered' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must reject uncommitted same-app window sentinel leakage'
require 'MultiAppSurface' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must inspect MultiAppSurface state'
require 'explicit_product_unsupported' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must require explicit unsupported state'
require 'unsupported multi-display app must not start capture session' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must reject unsupported capture start'
require 'remote_desktop\.create_session' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must inspect create_session evidence'
require 'remote_desktop\.attach' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must inspect attach evidence'
require 'remote_desktop\.watch_events' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must inspect watch_events evidence'
require 'remote_desktop\.end_session' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must inspect end_session evidence'
require 'terminal_receipt' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must inspect terminal receipt evidence'
require 'product_complete_claim.*False|product_complete_claim.*false' "$MULTI_WINDOW_TRACKING" \
  'multi-window tracking verifier must reject product completion claims'
require 'real_network_fallback_matrix' "$NETWORK_FALLBACK" \
  'network fallback verifier must require real network fallback proof mode'
require 'component_mock.*False|component_mock.*false' "$NETWORK_FALLBACK" \
  'network fallback verifier must reject component mock evidence'
require 'real_backend_runtime.*True|real_backend_runtime.*true' "$NETWORK_FALLBACK" \
  'network fallback verifier must require real backend/runtime evidence'
require 'direct' "$NETWORK_FALLBACK" \
  'network fallback verifier must require direct route evidence'
require 'stun_srflx' "$NETWORK_FALLBACK" \
  'network fallback verifier must require STUN srflx route evidence'
require 'turn_relay' "$NETWORK_FALLBACK" \
  'network fallback verifier must require TURN relay route evidence'
require 'easynet_relay' "$NETWORK_FALLBACK" \
  'network fallback verifier must require EasyNet relay route evidence'
require 'selected_candidate_pair' "$NETWORK_FALLBACK" \
  'network fallback verifier must inspect selected candidate-pair evidence'
require 'webrtc selected_resource_ura must bind selected Resource URA' "$NETWORK_FALLBACK" \
  'network fallback verifier must bind WebRTC evidence to selected Resource URA'
require 'webrtc session_id must bind session_id' "$NETWORK_FALLBACK" \
  'network fallback verifier must bind WebRTC evidence to RemoteApp session'
require 'caller_ura must identify an admitted User, Agent, or Authority' "$NETWORK_FALLBACK" \
  'network fallback verifier must keep Browser caller identity in the Invocation principal plane'
require 'callee_ura must identify the Remote Desktop SystemAgent' "$NETWORK_FALLBACK" \
  'network fallback verifier must require the Ability-owner SystemAgent as callee'
require 'provider_device_ura must identify the execution host Device' "$NETWORK_FALLBACK" \
  'network fallback verifier must keep Device in the execution-host plane'
require 'webrtc caller_ura must bind the admitted caller' "$NETWORK_FALLBACK" \
  'network fallback verifier must bind WebRTC evidence to the admitted caller'
require 'webrtc callee_ura must bind the SystemAgent callee' "$NETWORK_FALLBACK" \
  'network fallback verifier must bind WebRTC evidence to the SystemAgent callee'
require 'webrtc provider_device_ura must bind the execution host' "$NETWORK_FALLBACK" \
  'network fallback verifier must bind WebRTC evidence to the provider Device host'
require 'webrtc client_endpoint_id must bind the Browser/network peer' "$NETWORK_FALLBACK" \
  'network fallback verifier must identify the transport peer without promoting it to a Device'
require 'remote_desktop\.set_description' "$NETWORK_FALLBACK" \
  'network fallback verifier must require production WebRTC signalling'
require 'remote_desktop\.report_client_state' "$NETWORK_FALLBACK" \
  'network fallback verifier must require Browser transport-state reporting'
require 'webrtc route_kind must match scenario' "$NETWORK_FALLBACK" \
  'network fallback verifier must bind WebRTC evidence to route scenario'
require 'record_webrtc_diagnostic_projects_target_binding_context' "$SESSION" \
  'RemoteApp session tests must prove WebRTC diagnostic events carry selected target binding evidence'
require 'with_target_binding_context\(self\.target\.binding\(\), payload\)' "$SESSION" \
  'RemoteApp WebRTC diagnostic event path must attach selected target context before event-log commit'
require 'selected_candidate_pair\.selected must be true' "$NETWORK_FALLBACK" \
  'network fallback verifier must require selected ICE pair evidence'
require 'selected_candidate_pair\.nominated must be true' "$NETWORK_FALLBACK" \
  'network fallback verifier must require nominated ICE pair evidence'
require 'selected_candidate_pair\.state must be succeeded' "$NETWORK_FALLBACK" \
  'network fallback verifier must require succeeded ICE pair evidence'
require 'selected_candidate_pair\.candidate_pair_id must be recorded' "$NETWORK_FALLBACK" \
  'network fallback verifier must require selected candidate-pair id evidence'
require 'selected_candidate_pair\.local_candidate_id must be recorded' "$NETWORK_FALLBACK" \
  'network fallback verifier must require selected local candidate id evidence'
require 'selected_candidate_pair\.remote_candidate_id must be recorded' "$NETWORK_FALLBACK" \
  'network fallback verifier must require selected remote candidate id evidence'
require 'selected_route_class' "$NETWORK_FALLBACK" \
  'network fallback verifier must inspect selected route-class evidence'
require 'network_fixture' "$NETWORK_FALLBACK" \
  'network fallback verifier must require applied network fixture evidence'
require 'route_constraints_applied' "$NETWORK_FALLBACK" \
  'network fallback verifier must require route constraints to be applied'
require 'allowed_route_classes' "$NETWORK_FALLBACK" \
  'network fallback verifier must inspect allowed route classes'
require 'blocked_route_classes' "$NETWORK_FALLBACK" \
  'network fallback verifier must inspect blocked route classes'
require 'selected_pair_observed_at_ms must be after network constraints' "$NETWORK_FALLBACK" \
  'network fallback verifier must order selected-pair observation after network constraints'
require 'rendered_after_selected_pair' "$NETWORK_FALLBACK" \
  'network fallback verifier must require rendered media after selected pair'
require 'media selected_resource_ura must bind selected Resource URA' "$NETWORK_FALLBACK" \
  'network fallback verifier must bind rendered media to selected Resource URA'
require 'media session_id must bind session_id' "$NETWORK_FALLBACK" \
  'network fallback verifier must bind rendered media to RemoteApp session'
require 'media route_kind must match scenario' "$NETWORK_FALLBACK" \
  'network fallback verifier must bind rendered media to route scenario'
require 'media candidate_pair_id must match selected_candidate_pair' "$NETWORK_FALLBACK" \
  'network fallback verifier must bind rendered media to selected candidate pair'
require 'first_rendered_frame_at_ms must be after selected pair observation' "$NETWORK_FALLBACK" \
  'network fallback verifier must order rendered media after selected-pair observation'
require 'local_candidate_type' "$REPORT_CLIENT_STATE_HANDLER" \
  'authenticated Browser RTCStats must project selected local candidate type for network fallback evidence'
require 'remote_candidate_type' "$REPORT_CLIENT_STATE_HANDLER" \
  'authenticated Browser RTCStats must project selected remote candidate type for network fallback evidence'
require 'selected_route_class' "$REPORT_CLIENT_STATE_HANDLER" \
  'authenticated Browser RTCStats must project selected route class for network fallback evidence'
require '"protocol"' "$REPORT_CLIENT_STATE_HANDLER" \
  'authenticated Browser RTCStats must project selected candidate pair protocol for network fallback evidence'
require 'remote_desktop\.create_session' "$NETWORK_FALLBACK" \
  'network fallback verifier must inspect create_session evidence'
require 'remote_desktop\.set_description' "$NETWORK_FALLBACK" \
  'network fallback verifier must inspect production set_description evidence'
require 'remote_desktop\.report_client_state' "$NETWORK_FALLBACK" \
  'network fallback verifier must inspect authenticated Browser RTCStats evidence'
require 'remote_desktop\.watch_events' "$NETWORK_FALLBACK" \
  'network fallback verifier must inspect watch_events evidence'
require 'remote_desktop\.end_session' "$NETWORK_FALLBACK" \
  'network fallback verifier must inspect end_session evidence'
require 'frames_rendered' "$NETWORK_FALLBACK" \
  'network fallback verifier must inspect rendered media evidence'
require 'terminal_receipt' "$NETWORK_FALLBACK" \
  'network fallback verifier must inspect terminal receipt evidence'
require 'credentials_redacted' "$NETWORK_FALLBACK" \
  'network fallback verifier must require credential redaction'
require 'raw credential/secret fields are forbidden' "$NETWORK_FALLBACK" \
  'network fallback verifier must reject raw credential fields'
require 'product_complete_claim.*False|product_complete_claim.*false' "$NETWORK_FALLBACK" \
  'network fallback verifier must reject product completion claims'
require 'relay_allocation' "$NETWORK_FALLBACK" \
  'network fallback verifier must require relay allocation evidence'
require 'relay_only_policy_and_local_sdp' "$NETWORK_FALLBACK" \
  'network fallback verifier must bind relay selection to relay-only policy and local SDP'
require 'host-remoteapp-turn-relay-e2e\.sh' "$AUDIT" \
  'audit must record the reproducible TURN relay host runner'
require 'host-remoteapp-easynet-relay-e2e\.sh' "$AUDIT" \
  'audit must record the reproducible Hub-owned EasyNet relay host runner'
require 'host-remoteapp-direct-e2e\.sh' "$AUDIT" \
  'audit must record the reproducible direct-route host runner'
require 'host-remoteapp-stun-srflx-e2e\.sh' "$AUDIT" \
  'audit must record the reproducible STUN srflx host runner'
require '-u EASYNET_REMOTE_DESKTOP_STUN_URLS' "$DIRECT_ROUTE_RUNNER" \
  'direct-route host runner must remove daemon STUN configuration'
require '-u EASYNET_REMOTE_DESKTOP_TURN_URLS' "$DIRECT_ROUTE_RUNNER" \
  'direct-route host runner must remove daemon TURN configuration'
reject 'EASYNET_REMOTE_DESKTOP_EASYNET_RELAY_' "$REMOTE_DESKTOP_NETWORK" \
  'RemoteApp must obtain EasyNet relay credentials through the injected Hub lease port'
require 'trait RemoteDesktopRelayLeaseProvider' "$REMOTE_DESKTOP_RELAY_LEASE" \
  'RemoteApp relay acquisition must remain an injected product port'
require 'provider_with_relay_lease_provider' "$REMOTE_DESKTOP_EMBEDDED" \
  'RemoteApp package provider must accept the daemon-owned relay lease port'
require 'HubRemoteDesktopRelayLeaseProvider' "$DAEMON_PLUGINS" \
  'daemon composition root must inject the Hub relay lease adapter'
require 'load_credentials_optional' "$DAEMON_REMOTEAPP_RELAY" \
  'only the daemon relay adapter may load the durable device credential'
require 'HUB_RELAY_REQUEST_DEADLINE' "$DAEMON_REMOTEAPP_RELAY" \
  'Hub relay lease calls must remain time-bounded'
require '/api/v1/devices/relay-leases/acquire' "$DAEMON_REMOTEAPP_RELAY" \
  'daemon relay adapter must acquire leases from the Hub-owned endpoint'
require '/api/v1/devices/relay-leases/release' "$DAEMON_REMOTEAPP_RELAY" \
  'daemon relay adapter must release leases through the Hub-owned endpoint'
require 'schedule_relay_refresh' "$REMOTE_DESKTOP_LEASE_MONITOR" \
  'Hub relay refresh must remain in the single RemoteApp lease state machine'
require 'release_terminal_relay_lease' "$SESSION_LIFECYCLE" \
  'terminal RemoteApp settlement must release the ephemeral Hub relay lease'
require 'client_ice_servers' "$REMOTE_DESKTOP_VIEW_TRANSPORT" \
  'Browser transport projection must receive the session-owned relay ICE configuration'
require 'from_env_with_relay_lease' "$WEBRTC_ENDPOINT" \
  'device peer construction must receive the same session-owned relay lease'
require 'hub_relay_lease_reaches_both_ice_views_and_releases_after_terminal_commit' "$END_SESSION_HANDLER" \
  'RemoteApp must prove shared Browser/device relay configuration and post-commit release'
reject 'relay_lease|credential|username' "$SESSION_RECOVERY" \
  'RemoteApp recovery snapshots must not persist ephemeral relay credentials'
require 'EASYNET_REMOTEAPP_BROWSER_ICE_TRANSPORT_POLICY=all' "$DIRECT_ROUTE_RUNNER" \
  'direct-route host runner must exercise the ordinary Browser ICE policy'
require '--required-routes direct' "$DIRECT_ROUTE_RUNNER" \
  'direct-route host runner must emit a focused child proof'
require 'runtime start' "$DIRECT_ROUTE_RUNNER" \
  'direct-route host runner must restore the ordinary local daemon'
require 'daemon_zero_ice_servers_plus_host_only_sdp' "$NETWORK_SCENARIO_PROJECTOR" \
  'network scenario projector must bind direct proof to zero ICE URLs and host-only SDP'
require 'EASYNET_REMOTEAPP_BROWSER_ICE_TRANSPORT_POLICY=relay' "$TURN_RELAY_RUNNER" \
  'TURN relay host runner must constrain the real Browser to relay-only ICE'
require 'docker logs "\$CONTAINER"' "$TURN_RELAY_RUNNER" \
  'TURN relay host runner must collect server-side allocation evidence'
require '--required-routes turn_relay' "$TURN_RELAY_RUNNER" \
  'TURN relay host runner must emit a focused child proof'
require 'runtime start' "$TURN_RELAY_RUNNER" \
  'TURN relay host runner must restore the ordinary local daemon'
require 'EASYNET_RELAY_SHARED_SECRET' "$EASYNET_RELAY_RUNNER" \
  'EasyNet relay host runner must enable the Hub-owned ephemeral lease issuer'
require 'EASYNET_REMOTEAPP_BROWSER_ICE_TRANSPORT_POLICY=relay' "$EASYNET_RELAY_RUNNER" \
  'EasyNet relay host runner must constrain the real Browser to relay-only ICE'
reject 'EASYNET_REMOTE_DESKTOP_TURN_URLS=' "$EASYNET_RELAY_RUNNER" \
  'EasyNet relay host runner must not inject static daemon TURN credentials'
require '--route-kind easynet_relay' "$EASYNET_RELAY_RUNNER" \
  'EasyNet relay host runner must project a focused EasyNet relay proof'
require '--release-probe' "$EASYNET_RELAY_RUNNER" \
  'EasyNet relay host runner must prove terminal Hub lease release'
require '--refresh-resume' "$EASYNET_RELAY_RUNNER" \
  'EasyNet relay host runner must expose the accelerated refresh/resume scenario'
require 'EASYNET_REMOTEAPP_BROWSER_REQUIRE_RELAY_LEASE_REFRESH=1' "$EASYNET_RELAY_RUNNER" \
  'EasyNet relay refresh runner must require Browser-observed Hub lease rotation'
require 'EASYNET_REMOTEAPP_BROWSER_RELAY_REFRESH_READY_FILE' "$EASYNET_RELAY_RUNNER" \
  'EasyNet relay runner must not disconnect the daemon before Browser-observed lease rotation'
require '--relay-refresh' "$EASYNET_RELAY_RUNNER" \
  'EasyNet relay runner must bind verified refresh evidence into network projection'
require 'real_hub_relay_lease_refresh' "$EASYNET_RELAY_REFRESH_VERIFIER" \
  'EasyNet relay refresh verifier must require a real Hub/backend/runtime proof'
require 'same_public_session' "$EASYNET_RELAY_REFRESH_VERIFIER" \
  'EasyNet relay refresh verifier must preserve the public RemoteApp session'
require 'new_peer_connection' "$EASYNET_RELAY_REFRESH_VERIFIER" \
  'EasyNet relay refresh verifier must require replacement WebRTC transport'
require 'credentials_redacted' "$EASYNET_RELAY_REFRESH_VERIFIER" \
  'EasyNet relay refresh evidence must redact ephemeral relay credentials'
require 'relay_lease_refresh_resume' "$NETWORK_FALLBACK" \
  'network verifier must project relay refresh/resume coverage separately from route coverage'
require 'contract self-test must not claim live relay coverage' "$EASYNET_RELAY_RUNNER_TEST" \
  'EasyNet relay runner test must reject self-test evidence laundering'
require 'projector accepted relay refresh evidence from another session' "$EASYNET_RELAY_RUNNER_TEST" \
  'EasyNet relay runner test must attack cross-session refresh evidence'
require '--required-routes easynet_relay' "$EASYNET_RELAY_RUNNER" \
  'EasyNet relay host runner must validate the focused EasyNet relay proof'
require 'compose up -d --force-recreate hub' "$EASYNET_RELAY_RUNNER" \
  'EasyNet relay host runner must restore the ordinary Hub configuration'
require 'hub_lease_plus_browser_relay_only_policy_plus_coturn_allocation' "$NETWORK_SCENARIO_PROJECTOR" \
  'network projector must bind EasyNet relay selection to a Hub lease and server allocation'
require 'terminal_reacquire_rejected' "$NETWORK_SCENARIO_PROJECTOR" \
  'network projector must require a post-terminal Hub lease tombstone'
require 'terminal_reacquire_rejected' "$NETWORK_FALLBACK" \
  'network verifier must reject EasyNet relay evidence without terminal lease cleanup'
require 'Global turn allocation count incremented' "$NETWORK_SCENARIO_PROJECTOR" \
  'network scenario projector must parse a server-observed TURN allocation'
require 'TURN server did not report a relay allocation' "$NETWORK_SCENARIO_PROJECTOR" \
  'network scenario projector must fail closed when the TURN allocation is absent'
require 'remoteapp-stun-binding-server.py' "$STUN_SRFLX_RUNNER" \
  'STUN srflx host runner must start the bounded provider-host STUN fixture'
require 'stun-binding-events.jsonl' "$STUN_SRFLX_RUNNER" \
  'STUN srflx host runner must retain server-observed binding evidence'
require 'EASYNET_REMOTEAPP_STUN_E2E_BROWSER_DOCKER_CONTEXT' "$STUN_SRFLX_RUNNER" \
  'STUN srflx host runner must name its externally reachable VM-NAT Browser context'
require 'Docker Desktop has no reflexive ICE return route' "$STUN_SRFLX_RUNNER" \
  'STUN srflx host runner must reject the known non-routable macOS Docker Desktop topology'
require 'EASYNET_REMOTEAPP_BROWSER_ALLOWED_OUTBOUND_ICE_CANDIDATE_TYPES=srflx,prflx' "$STUN_SRFLX_RUNNER" \
  'STUN srflx host runner must block Browser host candidates on outbound signaling'
require 'EASYNET_REMOTEAPP_BROWSER_ALLOWED_INBOUND_ICE_CANDIDATE_TYPES=host,srflx,prflx' "$STUN_SRFLX_RUNNER" \
  'STUN srflx host runner must retain the provider host return candidate'
require 'verify_browser_candidate_boundary' "$STUN_SRFLX_RUNNER" \
  'STUN srflx host runner must verify the Browser signaling admission boundary'
require "admittedDescription\('outbound', super.localDescription\)" "$STUN_SRFLX_RUNNER" \
  'STUN srflx host runner must reject Browser code that leaks host candidates through local SDP'
require 'BROWSER_CONTAINER=' "$STUN_SRFLX_RUNNER" \
  'STUN srflx host runner must use a distinct Browser network namespace'
require 'BROWSER_RUN_DEADLINE_SECONDS' "$STUN_SRFLX_RUNNER" \
  'STUN srflx Browser child must have an outer execution deadline'
require '--required-routes stun_srflx' "$STUN_SRFLX_RUNNER" \
  'STUN srflx host runner must emit a focused child proof'
require 'runtime start' "$STUN_SRFLX_RUNNER" \
  'STUN srflx host runner must restore the ordinary local daemon'
require 'incoming packet BINDING processed, success' "$NETWORK_SCENARIO_PROJECTOR" \
  'network scenario projector must parse a server-observed STUN binding'
require 'easynet.remoteapp.stun-binding-event.v1' "$NETWORK_SCENARIO_PROJECTOR" \
  'network scenario projector must parse the address-redacted native STUN observer event'
require 'STUN server did not report a binding transaction' "$NETWORK_SCENARIO_PROJECTOR" \
  'network scenario projector must fail closed when the STUN binding is absent'
require 'browser_reflexive_outbound_plus_provider_host_return_and_server_binding' "$NETWORK_SCENARIO_PROJECTOR" \
  'network scenario projector must bind STUN proof to directional admission and server binding'
require 'credentials_redacted.*True|credentials_redacted.*true' "$NETWORK_SCENARIO_PROJECTOR" \
  'network scenario projector must emit explicit credential-redaction evidence'
require 'real_browser_tauri_lifecycle' "$FRONTEND_BROWSER_LIFECYCLE" \
  'frontend Browser/Tauri lifecycle verifier must require real lifecycle proof mode'
require 'expected_origin = "contract_self_test" if mode == "self-test" else "live_runner"' "$FRONTEND_BROWSER_LIFECYCLE" \
  'frontend Browser/Tauri lifecycle verifier must reject non-live evidence in run mode'
require 'evidence_origin.*contract_self_test' "$FRONTEND_BROWSER_LIFECYCLE" \
  'frontend Browser/Tauri lifecycle self-test must identify contract-only evidence'
require 'component_mock.*False|component_mock.*false' "$FRONTEND_BROWSER_LIFECYCLE" \
  'frontend Browser/Tauri lifecycle verifier must reject component mock evidence'
require 'real_backend_runtime.*True|real_backend_runtime.*true' "$FRONTEND_BROWSER_LIFECYCLE" \
  'frontend Browser/Tauri lifecycle verifier must require real backend/runtime evidence'
require 'target_picker_opened' "$FRONTEND_BROWSER_LIFECYCLE" \
  'frontend Browser/Tauri lifecycle verifier must inspect target picker evidence'
require 'evidence_source.*browser_automation.*tauri_automation|browser_automation.*tauri_automation' "$FRONTEND_BROWSER_LIFECYCLE" \
  'frontend Browser/Tauri lifecycle verifier must require real UI automation evidence source'
require 'component_snapshot_only must not be true' "$FRONTEND_BROWSER_LIFECYCLE" \
  'frontend Browser/Tauri lifecycle verifier must reject component snapshot-only evidence'
require 'observed_at_ms must be strictly increasing' "$FRONTEND_BROWSER_LIFECYCLE" \
  'frontend Browser/Tauri lifecycle verifier must require monotonic observed step timestamps'
require 'remote_desktop\.permission_status' "$FRONTEND_BROWSER_LIFECYCLE" \
  'frontend Browser/Tauri lifecycle verifier must inspect permission_status evidence'
require 'remote_desktop\.create_session' "$FRONTEND_BROWSER_LIFECYCLE" \
  'frontend Browser/Tauri lifecycle verifier must inspect create_session evidence'
require 'remote_desktop\.set_description' "$FRONTEND_BROWSER_LIFECYCLE" \
  'frontend Browser/Tauri lifecycle verifier must inspect production WebRTC signaling evidence'
require 'rtc_connection_state' "$FRONTEND_BROWSER_LIFECYCLE" \
  'frontend Browser/Tauri lifecycle verifier must require connected WebRTC state'
require 'media_stream_attached' "$FRONTEND_BROWSER_LIFECYCLE" \
  'frontend Browser/Tauri lifecycle verifier must require attached media stream evidence'
require 'remote_desktop\.watch_events' "$FRONTEND_BROWSER_LIFECYCLE" \
  'frontend Browser/Tauri lifecycle verifier must inspect watch_events evidence'
require 'media_element_visible' "$FRONTEND_BROWSER_LIFECYCLE" \
  'frontend Browser/Tauri lifecycle verifier must require visible media element evidence'
require 'frames_presented' "$FRONTEND_BROWSER_LIFECYCLE" \
  'frontend Browser/Tauri lifecycle verifier must require rendered frame count evidence'
require 'visible_status' "$FRONTEND_BROWSER_LIFECYCLE" \
  'frontend Browser/Tauri lifecycle verifier must require visible input status evidence'
require 'blocked_reason' "$FRONTEND_BROWSER_LIFECYCLE" \
  'frontend Browser/Tauri lifecycle verifier must require explicit input policy-block reason'
require 'remote_desktop\.end_session' "$FRONTEND_BROWSER_LIFECYCLE" \
  'frontend Browser/Tauri lifecycle verifier must inspect end_session evidence'
require 'terminal_receipt_visible' "$FRONTEND_BROWSER_LIFECYCLE" \
  'frontend Browser/Tauri lifecycle verifier must inspect terminal receipt visibility'
require 'input_applied target_focus_epoch must be positive' "$FRONTEND_BROWSER_LIFECYCLE" \
  'frontend Browser/Tauri lifecycle verifier must require applied input focus epoch'
require 'submitted_frame target_focus_epoch must match input_applied target_focus_epoch' "$FRONTEND_BROWSER_LIFECYCLE" \
  'frontend Browser/Tauri lifecycle verifier must bind submitted input frame to target focus epoch'
require 'applied_event target_focus_epoch must match input_applied target_focus_epoch' "$FRONTEND_BROWSER_LIFECYCLE" \
  'frontend Browser/Tauri lifecycle verifier must bind daemon applied event to target focus epoch'
require 'product_complete_claim.*False|product_complete_claim.*false' "$FRONTEND_BROWSER_LIFECYCLE" \
  'frontend Browser/Tauri lifecycle verifier must reject product completion claims'
require 'remote_desktop\.show_session' "$SESSION_TIMEOUT" \
  'session timeout E2E must observe timeout through public show_session'
require 'remote_desktop\.end_session' "$SESSION_TIMEOUT" \
  'session timeout E2E must prove post-timeout end_session idempotency'
require 'session_expired' "$SESSION_TIMEOUT" \
  'session timeout E2E must prove session_expired terminal reason'
require 'terminal_receipt\.reason_code' "$SESSION_TIMEOUT" \
  'session timeout E2E must inspect timeout terminal_receipt.reason_code'
require 'remote_desktop\.end_session' "$SESSION_CANCEL" \
  'session cancel E2E must invoke public end_session'
require 'remote_desktop\.show_session' "$SESSION_CANCEL" \
  'session cancel E2E must observe cancel through public show_session'
require 'user_cancelled' "$SESSION_CANCEL" \
  'session cancel E2E must prove user_cancelled terminal reason'
require 'terminal_receipt\.reason_code' "$SESSION_CANCEL" \
  'session cancel E2E must inspect cancel terminal_receipt.reason_code'
require 'end_cancel_again must be idempotent after user cancel' "$SESSION_CANCEL" \
  'session cancel E2E must inspect repeated end_session idempotency'
require 'end_cancel_again must preserve the original cancel terminal receipt' "$SESSION_CANCEL" \
  'session cancel E2E must prove repeated end_session preserves the terminal receipt'
require 'real_platform_permission_revoke' "$PERMISSION_REVOKE" \
  'permission revoke E2E must require real platform revoke proof mode'
require 'operator_revoke_required.*True|operator_revoke_required.*true' "$PERMISSION_REVOKE" \
  'permission revoke E2E must require operator/platform revoke'
require 'remote_desktop\.show_session' "$PERMISSION_REVOKE" \
  'permission revoke E2E must observe revoke through public show_session'
require 'target_permission_revoked' "$PERMISSION_REVOKE" \
  'permission revoke E2E must prove target_permission_revoked terminal reason'
require 'TARGET_PERMISSION_REVOKED' "$PERMISSION_REVOKE" \
  'permission revoke E2E must inspect TARGET_PERMISSION_REVOKED event evidence'
require 'MEDIA_SOURCE_LOST' "$PERMISSION_REVOKE" \
  'permission revoke E2E must inspect MEDIA_SOURCE_LOST event evidence'
require 'SESSION_CLOSED' "$PERMISSION_REVOKE" \
  'permission revoke E2E must inspect SESSION_CLOSED event evidence'
require 'terminal_receipt\.reason_code' "$PERMISSION_REVOKE" \
  'permission revoke E2E must inspect revoke terminal_receipt.reason_code'
require 'remote_desktop\.refresh_lease' "$SESSION_RESUME" \
  'session resume E2E must invoke public refresh_lease'
require 'remote_desktop\.show_session' "$SESSION_RESUME" \
  'session resume E2E must validate through public show_session'
require 'lease_refresh_resume' "$SESSION_RESUME" \
  'session resume E2E must identify lease_refresh_resume proof mode'
require 'waited_past_original_lease' "$SESSION_RESUME" \
  'session resume E2E must wait past original lease'
require 'refresh_lease must extend lease_expires_at_ms' "$SESSION_RESUME" \
  'session resume E2E must prove lease extension'
require 'show_after_original_lease must prove the refreshed session survived' "$SESSION_RESUME" \
  'session resume E2E must prove same-session survival after original lease'
require 'resume_e2e_cleanup' "$SESSION_RESUME" \
  'session resume E2E must clean up with explicit terminal reason'
require 'real_crash_restart_recovery_matrix' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must require real recovery proof mode'
require 'expected_origin = "contract_self_test" if mode == "self-test" else "live_runner"' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must reject non-live evidence in run mode'
require 'evidence_origin.*contract_self_test' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery self-test must identify contract-only evidence'
require 'component_mock.*False|component_mock.*false' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must reject component mock evidence'
require 'real_backend_runtime.*True|real_backend_runtime.*true' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must require real backend/runtime evidence'
require 'daemon_restart_active_session' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must require daemon active-session restart scenario'
require 'plugin_worker_restart' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must require plugin worker restart scenario'
require 'terminal_receipt_replay_after_crash' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must require terminal receipt replay scenario'
require 'stale_socket_restart_cleanup' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must require stale socket cleanup scenario'
require 'PROCESS_STOPPED_UNCLEAN' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must inspect unclean process stop evidence'
require 'DAEMON_RESTARTED' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must inspect daemon restart evidence'
require 'SESSION_REHYDRATED' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must inspect session rehydration evidence'
require 'scenario_started_at_ms must be recorded' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must require scenario start timestamp evidence'
require 'events must be strictly ordered by at_ms' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must require ordered lifecycle events'
require 'selected_resource_ura must bind selected Resource URA' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must bind lifecycle events to selected Resource URA'
require 'session_id must bind session_id' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must bind lifecycle events to the session'
require 'PROCESS_STOPPED_UNCLEAN must occur before DAEMON_RESTARTED' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must require daemon restart event ordering'
require 'DAEMON_RESTARTED must occur before SESSION_REHYDRATED' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must require session rehydration after daemon restart'
require 'PLUGIN_WORKER_RESTARTED' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must inspect plugin worker restart evidence'
require 'PLUGIN_WORKER_CRASHED must occur before PLUGIN_WORKER_RESTARTED' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must require plugin worker restart event ordering'
require 'PLUGIN_WORKER_RESTARTED must occur before TARGET_MONITOR_RESTARTED' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must require target monitor restart after plugin worker restart'
require 'TERMINAL_RECEIPT_REPLAYED' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must inspect terminal receipt replay evidence'
require 'END_SESSION_ACCEPTED must occur before PROCESS_STOPPED_UNCLEAN' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must require end-session acceptance before crash'
require 'PROCESS_STOPPED_UNCLEAN must occur before TERMINAL_RECEIPT_REPLAYED' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must require terminal receipt replay after crash'
require 'STALE_CONTROL_SOCKET_DETECTED' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must inspect stale control socket evidence'
require 'STALE_INVOCATION_SOCKET_DETECTED' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must inspect stale invocation socket evidence'
require 'idempotency_state_recovered' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must require idempotency recovery evidence'
require 'replay_guard_recovered must be true' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must require replay guard recovery'
require 'lock_owner_recovered' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must require lock owner recovery'
require 'must remain stable across restart' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must reject public session replacement'
require 'watch_events_reattached' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must require watch_events reattachment'
require 'watch_events_reattached_at_ms must be after SESSION_REHYDRATED' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must require watch_events reattachment after rehydration'
require 'media_reattached' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must require media reattachment'
require 'media_reattached_at_ms must be after watch_events_reattached_at_ms' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must require media reattachment after watch_events'
require 'frames_rendered_after_restart must be positive' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must require post-restart rendered media'
require 'first_frame_rendered_after_restart_at_ms must be after media_reattached_at_ms' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must require rendered media after media reattachment'
require 'first_frame_rendered_after_worker_restart_at_ms must be after TARGET_MONITOR_RESTARTED' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must require rendered media after plugin worker recovery'
require 'terminal event identity must be replayed' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must require original terminal event identity replay'
require 'show_session_after_restart_observed_at_ms must be after TERMINAL_RECEIPT_REPLAYED' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must require public show_session after receipt replay'
require 'endpoint_ready_at_ms must be after DAEMON_READY_AFTER_RESTART' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must require endpoint readiness after daemon-ready event'
require 'manual_cleanup_required.*False|manual_cleanup_required.*false' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must reject manual stale-socket cleanup'
require 'remote_desktop\.create_session' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must inspect create_session evidence'
require 'remote_desktop\.show_session' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must inspect show_session evidence'
require 'remote_desktop\.watch_events' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must inspect watch_events evidence'
require 'remote_desktop\.end_session' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must inspect end_session evidence'
require 'terminal_receipt' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must inspect terminal receipt evidence'
require 'real_target_monitor_worker_only_recovery' "$TARGET_MONITOR_WORKER_RECOVERY" \
  'target-monitor worker runner must identify the real worker-only proof mode'
require 'worker-only recovery must preserve daemon pid' "$TARGET_MONITOR_WORKER_RECOVERY" \
  'target-monitor worker runner must require one stable daemon process'
require 'capture_stable_runtime_status' "$TARGET_MONITOR_WORKER_RECOVERY" \
  'target-monitor worker runner must wait for stable J800 baselines'
require 'remoteapp-e2e-fault-injection' "$TARGET_MONITOR_WORKER_RECOVERY" \
  'target-monitor worker fault must remain behind the explicit E2E feature'
require 'worker_event_records' "$TARGET_MONITOR_WORKER_RECOVERY" \
  'target-monitor worker runner must inspect public Browser event records'
require 'persisted_events' "$TARGET_MONITOR_WORKER_RECOVERY" \
  'target-monitor worker runner must bind durable lifecycle event evidence'
require 'restarted generation must increase' "$TARGET_MONITOR_WORKER_RECOVERY" \
  'target-monitor worker runner must require an increasing replacement generation'
require 'Browser must render a later frame' "$TARGET_MONITOR_WORKER_RECOVERY" \
  'target-monitor worker runner must require post-recovery Browser media'
require 'worker recovery must not request new consent' "$TARGET_MONITOR_WORKER_RECOVERY" \
  'target-monitor worker runner must preserve session consent'
require 'cargo build --quiet --bin easynet --bin easynet-daemon' "$TARGET_MONITOR_WORKER_RECOVERY" \
  'target-monitor worker runner must restore ordinary product binaries'
require 'after_pid.*\+= 1' "$TARGET_MONITOR_WORKER_RECOVERY_TEST" \
  'target-monitor worker contract test must reject daemon process replacement'
require 'media_source_epoch_after.*\+= 1' "$TARGET_MONITOR_WORKER_RECOVERY_TEST" \
  'target-monitor worker contract test must reject media-source replacement'
require 'restarted_generation.*= 7' "$TARGET_MONITOR_WORKER_RECOVERY_TEST" \
  'target-monitor worker contract test must reject non-increasing generations'
require 'product_complete_claim.*False|product_complete_claim.*false' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must reject product completion claims'
require 'RemoteDesktopRecoverySnapshot' "$SESSION_RECOVERY" \
  'session recovery store must define a versioned durable snapshot contract'
require 'RemoteDesktopRecoveryStore' "$SESSION_RECOVERY" \
  'session recovery store must define the daemon-local durable store'
require 'from_session' "$SESSION_RECOVERY" \
  'session recovery snapshot must be derived from the canonical session aggregate'
require 'daemon_default' "$SESSION_RECOVERY" \
  'session recovery store must have a daemon-local default state path'
require 'load_all' "$SESSION_RECOVERY" \
  'session recovery store must enumerate snapshots for plugin startup rehydration'
require 'RemoteDesktopRecoveryLoadReport' "$SESSION_RECOVERY" \
  'session recovery batch loading must return accepted snapshots plus rejected-row evidence'
require 'RemoteDesktopRecoveryLoadRejection' "$SESSION_RECOVERY" \
  'session recovery batch loading must identify corrupt or mismatched snapshots without poisoning valid rows'
require 'harden_recovery_dir' "$SESSION_RECOVERY" \
  'session recovery store must harden daemon-local recovery directories because snapshots include session tokens'
require 'atomic_write_with_permissions' "$SESSION_RECOVERY" \
  'session recovery store must use the shared unique-temp fsync and atomic-rename writer'
require 'WritePermissions::OwnerReadWrite' "$SESSION_RECOVERY" \
  'session recovery staging and final files must be owner-only because snapshots include session tokens'
require 'ExclusiveFileLock::acquire_for_data_path' "$SESSION_RECOVERY" \
  'session recovery commits and deletes must serialize through one store lock across daemon processes'
require 'MAX_RECOVERY_SNAPSHOT_BYTES' "$SESSION_RECOVERY" \
  'session recovery snapshots must have an explicit hard byte bound'
require 'take\(MAX_RECOVERY_SNAPSHOT_BYTES \+ 1\)' "$SESSION_RECOVERY" \
  'session recovery reads must enforce the byte bound before JSON decode'
require 'serialize_snapshot_bounded' "$SESSION_RECOVERY" \
  'session recovery writes must enforce the byte bound during serialization'
require 'MAX_RECOVERY_BATCH_BYTES' "$SESSION_RECOVERY" \
  'session recovery startup must have an explicit aggregate byte bound'
require 'max_directory_entries' "$SESSION_RECOVERY" \
  'session recovery startup must bound all directory entries, including non-snapshot files'
require 'max_session_rows_for_active_limit' "$RUNTIME" \
  'session recovery startup row capacity must derive from the canonical session retention policy'
require 'recovery_store_rejects_snapshot_count_above_session_retention_bound' "$SESSION_RECOVERY" \
  'session recovery tests must reject snapshot cardinality above the session retention bound'
require 'recovery_store_rejects_unbounded_non_snapshot_directory_entries' "$SESSION_RECOVERY" \
  'session recovery tests must reject a non-snapshot directory-entry storm'
require 'recovery_store_rejects_batch_bytes_before_json_decode' "$SESSION_RECOVERY" \
  'session recovery tests must reject oversized recovery batches before JSON decode'
require 'fn delete' "$SESSION_RECOVERY" \
  'session recovery store must expose durable deletion for pruned tombstones'
require 'create_session_deletes_terminal_recovery_rows_pruned_from_memory' "$CREATE_SESSION_HANDLER" \
  'session retention tests must prove memory pruning also deletes durable snapshots'
require 'recovery_snapshot_should_replace' "$SESSION_RECOVERY" \
  'session recovery commits must reject stale active snapshots after terminal publication'
require 'session_token' "$SESSION_RECOVERY" \
  'session recovery snapshot must preserve daemon-local session token for post-restart control access'
require 'schema_version' "$SESSION_RECOVERY" \
  'session recovery snapshot must be schema-versioned'
require 'selected_resource_ura' "$SESSION_RECOVERY" \
  'session recovery snapshot must bind the selected Resource URA'
require 'terminal_receipt' "$SESSION_RECOVERY" \
  'session recovery snapshot must preserve terminal receipt projection'
require 'input_runtime_block_reason' "$SESSION_RECOVERY" \
  'session recovery snapshot must preserve runtime input permission blockers'
require '#\[serde\(default\)\]' "$SESSION_RECOVERY" \
  'session recovery snapshot optional runtime input blocker field must keep legacy rows loadable'
require 'recovery_snapshot_round_trips_runtime_input_block_reason' "$SESSION_RECOVERY" \
  'session recovery tests must prove runtime input blockers round-trip durably'
require 'recovery_snapshot_keeps_legacy_rows_without_runtime_input_block_reason_loadable' "$SESSION_RECOVERY" \
  'session recovery tests must prove old snapshots without runtime input blocker still load'
require 'persist_recovery_snapshot' "$RUNTIME" \
  'RemoteApp runtime must expose a plugin-owned recovery snapshot write boundary'
require 'rehydrate_recovery_snapshots' "$RUNTIME" \
  'RemoteApp runtime must load recovery snapshots into the plugin session store at startup'
require 'ignored recovery snapshot' "$RUNTIME" \
  'RemoteApp runtime startup must report and skip corrupt recovery snapshots without failing the whole batch'
require 'track_session_target\(plugin, session_id\.clone\(\)\)' "$RUNTIME" \
  'RemoteApp runtime startup must re-register rehydrated non-terminal sessions with target monitoring'
require 'is_expired_at\(recovery_now_ms\)' "$RUNTIME" \
  'RemoteApp runtime startup must synchronously settle recovery snapshots whose leases expired while the daemon was down'
require 'session\.expire\(recovery_now_ms\)' "$RUNTIME" \
  'RemoteApp runtime startup must project expired recovery rows through the session aggregate terminal path'
require 'plugin_startup_rehydrates_recovery_snapshot_for_public_show_session' "$RUNTIME" \
  'RemoteApp runtime must have regression coverage for startup rehydrate show/watch/end behavior'
require 'shown\["input_readiness"\]\["blocked_reason"\]' "$RUNTIME" \
  'RemoteApp startup recovery regression must prove public show_session preserves runtime input blockers'
require 'plugin_startup_expires_recovery_snapshot_that_lapsed_while_daemon_was_down' "$RUNTIME" \
  'RemoteApp runtime must have regression coverage for startup recovery expiry settlement'
require 'target_monitor_desired_sessions_for_test' "$RUNTIME" \
  'RemoteApp runtime regression must prove rehydrated sessions re-enter target monitoring'
require 'desired: Arc<Mutex<HashSet<String>>>' "$TARGET_MONITOR" \
  'RemoteApp target monitor must keep plugin-owned desired tracking state outside the worker thread'
require 'initial_tracked: HashSet<String>' "$TARGET_MONITOR" \
  'RemoteApp target monitor worker restarts must be seeded from desired tracking state'
require 'desired_sessions_for_test' "$TARGET_MONITOR" \
  'RemoteApp target monitor must expose test evidence for desired tracking state'
require 'struct TargetSnapshotDeadlineExecutor' "$TARGET_SNAPSHOT" \
  'RemoteApp target monitor must own a single-flight native snapshot deadline boundary'
require 'snapshot_deadline_fences_late_result_and_bounds_native_call_count' "$TARGET_MONITOR" \
  'RemoteApp target monitor must prove late native results cannot cross generation authority'
require 'provider_hang_exhausts_budget_without_spawning_unbounded_native_calls' "$TARGET_MONITOR" \
  'RemoteApp target monitor must prove provider hangs are bounded and fail safe'
require 'struct ManagedDiagnosticPreview' "$TRANSPORT_MANAGER" \
  'RemoteApp transport manager must own diagnostic preview generations'
require 'completion: Receiver<PreviewTaskGroupCompletion>' "$TRANSPORT_MANAGER" \
  'RemoteApp diagnostic preview ownership must retain a worker-group completion receipt'
require 'activate_preview' "$TRANSPORT_MANAGER" \
  'RemoteApp transport manager must activate preview generations through one ownership boundary'
require 'stale_preview_activation_cannot_replace_newer_generation' "$TRANSPORT_MANAGER" \
  'RemoteApp preview manager must prove stale activation cannot replace a newer generation'
require 'retired_preview_settlement_waits_for_worker_group_completion' "$TRANSPORT_MANAGER" \
  'RemoteApp preview manager must prove terminal settlement waits for worker completion'
require 'disconnected_preview_completion_cannot_prove_settlement' "$TRANSPORT_MANAGER" \
  'RemoteApp preview manager must reject channel disconnect as worker completion'
require 'TRANSPORT_SETTLEMENT_DEADLINE' "$TRANSPORT_MANAGER" \
  'RemoteApp process-local transport settlement must have one bounded deadline'
require 'direct_endpoint_settlement_is_bounded_when_worker_does_not_exit' "$TRANSPORT_MANAGER" \
  'RemoteApp direct WebRTC settlement must prove a hung platform worker cannot hang shutdown'
require 'enum TransportSettlementStatus' "$TRANSPORT_MANAGER" \
  'RemoteApp transport settlement must distinguish Settled, Pending, and Failed'
require 'pending_reservation_timeout_retains_completion_ownership' "$TRANSPORT_MANAGER" \
  'RemoteApp pending endpoint timeout must retain completion ownership for later settlement'
require 'dropped_reservation_before_terminal_remains_manager_visible' "$TRANSPORT_MANAGER" \
  'RemoteApp dropped setup reservations must remain visible to a later terminal sweep'
require 'negative_receipt_does_not_discard_remaining_transport_ownership' "$TRANSPORT_MANAGER" \
  'RemoteApp negative settlement receipts must not discard other reservation or worker ownership'
require 'struct DirectWebRtcEndpointReservation' "$TRANSPORT_MANAGER" \
  'RemoteApp direct WebRTC setup must retain a manager-owned pending reservation'
require 'state\.sealed = true;' "$TRANSPORT_MANAGER" \
  'RemoteApp terminal transport sweep must seal direct WebRTC endpoint admission'
require 'pending: VecDeque<Receiver<DirectWebRtcReservationCompletion>>' "$TRANSPORT_MANAGER" \
  'RemoteApp terminal settlement must include pending direct WebRTC setup completion'
require 'terminal_seal_cancels_and_settles_pending_endpoint_reservation' "$TRANSPORT_MANAGER" \
  'RemoteApp transport manager must prove terminal seal cancels and settles pending endpoint setup'
require 'high_watermark: Option<TransportEpoch>' "$TRANSPORT_MANAGER" \
  'RemoteApp direct WebRTC admission must fence endpoint generations by a monotonic high-watermark'
require 'newer_endpoint_reservation_cancels_and_fences_older_generation' "$TRANSPORT_MANAGER" \
  'RemoteApp transport manager must prove an older reservation cannot replace a newer endpoint generation'
require 'reserve_endpoint\(session_id\.clone\(\), epoch\)' "$WEBRTC_ENDPOINT" \
  'RemoteApp direct WebRTC endpoint setup must reserve transport admission before construction'
require 'reservation\.commit\(' "$WEBRTC_ENDPOINT" \
  'RemoteApp direct WebRTC endpoint activation must atomically commit its pending reservation'
require 'complete_with_endpoint_cleanup' "$TRANSPORT_MANAGER" \
  'RemoteApp partially-created WebRTC peers must transfer into a retained cleanup owner'
require 'endpoint_cleanup_remains_visible_to_concurrent_terminal_settlement' "$TRANSPORT_MANAGER" \
  'RemoteApp terminal settlement must prove setup cleanup remains manager-visible until its real close receipt'
require 'struct EndpointSetupCleanupJob' "$TRANSPORT_MANAGER" \
  'RemoteApp setup cleanup must be owned by the canonical settlement executor'
require 'struct TransportSettlementQueue' "$TRANSPORT_MANAGER" \
  'RemoteApp transport ownership must converge on one process-owned settlement queue'
require 'easynet-rd-settlement-executor' "$TRANSPORT_MANAGER" \
  'RemoteApp transport manager must bootstrap one canonical settlement executor'
require 'static TRANSPORT_CLEANUP_RUNTIME: OnceLock<tokio::runtime::Runtime>' "$TRANSPORT_MANAGER" \
  'RemoteApp setup cleanup runtime must have process lifetime'
require 'static TRANSPORT_RUNTIME: OnceLock<Result<tokio::runtime::Runtime, String>>' "$TRANSPORT_MANAGER" \
  'RemoteApp WebRTC runtime must have process lifetime'
require 'settlement_cleanup_runtime_outlives_manager_while_job_pending' "$TRANSPORT_MANAGER" \
  'RemoteApp setup cleanup runtime must outlive manager shutdown while cleanup remains pending'
require 'settlement_executor_quarantines_panicking_job_without_dropping_owner' "$TRANSPORT_MANAGER" \
  'RemoteApp settlement executor must prove a failed job is quarantined without owner loss'
require 'executor shutdown cannot drop quarantined transport ownership' "$TRANSPORT_MANAGER" \
  'RemoteApp settlement quarantine must outlive every manager submission handle'
require 'struct QuarantinedTransportSettlementJob' "$TRANSPORT_MANAGER" \
  'RemoteApp settlement failures must retain typed job, identity, retry, and projection state'
require 'struct SettlementAdmissionGate' "$TRANSPORT_MANAGER" \
  'RemoteApp settlement quarantine and session creation must share one admission linearization gate'
require 'struct TransportSettlementAdmissionPermit' "$TRANSPORT_MANAGER" \
  'RemoteApp session creation must retain an admission permit through its bounded insert'
require 'acquire_session_admission' "$CREATE_SESSION_HANDLER" \
  'RemoteApp create_session must acquire the manager-owned settlement admission permit'
require 'admission_permit_linearizes_before_quarantine_projection' "$TRANSPORT_MANAGER" \
  'RemoteApp admission tests must prove quarantine waits for an already-admitted insert boundary'
require 'fn project_quarantine' "$TRANSPORT_MANAGER" \
  'RemoteApp settlement jobs must expose an explicit quarantine outcome projection boundary'
require 'run_transport_quarantine_projector' "$TRANSPORT_MANAGER" \
  'RemoteApp quarantine outcomes must run on a dedicated projector outside submitting session locks'
require 'executor_unavailable_never_projects_quarantine_on_submitter' "$TRANSPORT_MANAGER" \
  'RemoteApp executor-unavailable fallback must prove it cannot re-enter session state on the submitter'
require 'endpoint_cleanup_quarantine_emits_negative_completion_receipt' "$TRANSPORT_MANAGER" \
  'RemoteApp quarantined endpoint cleanup must prove its parent receives an explicit negative receipt'
require 'failed_settlement_closes_admission_and_projects_typed_health' "$TRANSPORT_MANAGER" \
  'RemoteApp settlement quarantine must close admission and expose typed health'
require 'quarantine_projection_retries_after_queue_becomes_idle' "$TRANSPORT_MANAGER" \
  'RemoteApp idle settlement executor must retry failed quarantine outcome projections'
require 'next_quarantine_projection_at' "$TRANSPORT_MANAGER" \
  'RemoteApp quarantine retries must sleep until the exact next projection attempt'
require 'create_session_fails_closed_while_transport_quarantine_owns_resources' "$CREATE_SESSION_HANDLER" \
  'RemoteApp create_session must fail closed while quarantined resources remain owned'
require 'quarantined_session_termination_publishes_durable_failed_outcome' "$SESSION_LIFECYCLE" \
  'RemoteApp quarantined session termination must publish a durable Failed outcome'
require 'transport_settlement_health' "$SHOW_SESSION_HANDLER" \
  'RemoteApp session audit view must expose transport settlement health to operators and frontend'
require 'thread::sleep\(TRANSPORT_SETTLEMENT_POLL_INTERVAL\);' "$TRANSPORT_MANAGER" \
  'RemoteApp settlement executor must pace pending jobs instead of busy-spinning'
require 'fn next_poll_at\(&self\) -> Option<Instant>' "$TRANSPORT_MANAGER" \
  'RemoteApp settlement jobs must expose an explicit delayed-retry scheduling boundary'
require 'delayed_pending_job_is_not_polled_before_its_ready_time' "$TRANSPORT_MANAGER" \
  'RemoteApp settlement executor must prove persistence backoff does not become empty polling'
reject 'easynet-rd-(preview-reaper|endpoint-reaper|setup-reaper)' "$TRANSPORT_MANAGER" \
  'RemoteApp transport manager must not spawn per-resource settlement reaper threads'
reject 'easynet-rd-(session-transport-reaper|session-settler|expiration-settler)' "$SESSION_LIFECYCLE" \
  'RemoteApp session lifecycle must use the canonical settlement executor instead of ad-hoc threads'
require '"peer construction"' "$WEBRTC_ENDPOINT" \
  'RemoteApp WebRTC peer construction must run inside the cancellable setup deadline'
require '"description and media setup"' "$WEBRTC_ENDPOINT" \
  'RemoteApp WebRTC description/media setup must share the same absolute deadline'
require 'endpoint_setup_phase_is_interrupted_by_terminal_admission_cancel' "$WEBRTC_ENDPOINT" \
  'RemoteApp WebRTC setup must prove terminal cancellation interrupts a pending phase'
require 'endpoint_setup_phase_enforces_one_absolute_deadline' "$WEBRTC_ENDPOINT" \
  'RemoteApp WebRTC setup must prove a hung phase is bounded by one absolute deadline'
require 'transports: Weak<RemoteDesktopTransportManager>' "$WEBRTC_CALLBACKS" \
  'RemoteApp PeerConnection callbacks must not strongly retain the transport manager'
require 'peer_connection_handler_does_not_keep_transport_manager_alive' "$WEBRTC_CALLBACKS" \
  'RemoteApp WebRTC callbacks must prove the manager-peer-handler ownership cycle is absent'
require 'transport_runtime = endpoint_config\.transports\.runtime_handle\(\)' "$WEBRTC_ENDPOINT" \
  'RemoteApp media worker must retain only an independent runtime handle'
reject 'transports\.block_on\(run_direct_webrtc_media_loop' "$WEBRTC_ENDPOINT" \
  'RemoteApp media worker must not strongly retain the transport manager'
require 'tokio::join!\(control, forwarder, frame_source\)' "$INVOKE_BIDI" \
  'RemoteApp diagnostic preview must aggregate control, forwarding, and frame workers into one completion receipt'
require 'changed = stop_rx\.changed\(\)' "$INVOKE_BIDI" \
  'RemoteApp diagnostic control worker must observe session stop independently of client input'
require 'async fn send_bidi_output_or_stop' "$INVOKE_BIDI" \
  'RemoteApp diagnostic output must share one stop-aware bounded-queue publication boundary'
require 'permit = to_client\.reserve\(\)' "$INVOKE_BIDI" \
  'RemoteApp diagnostic output must reserve queue capacity without surrendering stop cancellation'
require 'const BIDI_TERMINAL_SEND_DEADLINE: Duration' "$BIDI_TERMINAL" \
  'RemoteApp diagnostic terminal publication must have a bounded client-backpressure deadline'
require '\.try_send\(BidiOutputFrame::json' "$BIDI_TERMINAL" \
  'RemoteApp blocking capture workers must not block teardown on a full client queue'
require 'bidi_terminal_guard_does_not_block_shutdown_on_full_client_queue' "$BIDI_TERMINAL" \
  'RemoteApp terminal guard must prove client backpressure cannot own worker settlement'
require 'activate_preview\(' "$ATTACH_HANDLER" \
  'RemoteApp attach must transfer preview stop and completion ownership to the transport manager'
require 'completion_rx' "$ATTACH_HANDLER" \
  'RemoteApp attach must reserve manager completion ownership before starting preview workers'
require 'attach_reserves_preview_ownership_before_worker_can_race_session_close' "$ATTACH_HANDLER" \
  'RemoteApp attach must prove close cannot overtake preview ownership registration'
require 'struct RetiredSessionTransports' "$SESSION_LIFECYCLE" \
  'RemoteApp terminal lifecycle must settle production and diagnostic transports as one owned unit'
require 'diagnostic_preview: Option<RetiredDiagnosticPreview>' "$SESSION_LIFECYCLE" \
  'RemoteApp terminal settlement must include diagnostic preview completion'
require 'fn settlement_status_until' "$SESSION_LIFECYCLE" \
  'RemoteApp terminal settlement must combine preview and direct WebRTC completion as one state machine'
require 'settle_session_transports_and_finish' "$SESSION_LIFECYCLE" \
  'RemoteApp deferred settlement must retain ownership and own the terminal session commit'
require 'deferred_settler_retains_ownership_and_finishes_after_initial_timeout' "$SESSION_LIFECYCLE" \
  'RemoteApp terminal lifecycle must prove a timeout retains ownership and later commits Closed'
require 'terminal_persistence_failure_retains_closing_until_retry_commits' "$SESSION_LIFECYCLE" \
  'RemoteApp terminal lifecycle must retry durable publication after a persistence fault'
require 'commit_retry_delay' "$SESSION_LIFECYCLE" \
  'RemoteApp terminal persistence retries must use bounded exponential backoff'
require 'fn next_poll_at\(&self\) -> Option<Instant>' "$SESSION_LIFECYCLE" \
  'RemoteApp terminal persistence backoff must be scheduled by the settlement executor'
require 'terminal candidate revision changed after durable staging; retry required' "$SESSION_LIFECYCLE" \
  'RemoteApp terminal publication must use a revision CAS after durable staging'
require 'RemoteDesktopRecoveryStagedSnapshot' "$SESSION_RECOVERY" \
  'RemoteApp recovery must represent staged terminal candidates as non-authoritative state'
require 'let staged = recovery\.stage\(snapshot\)\?;' "$SESSION_LIFECYCLE" \
  'RemoteApp terminal serialization and file I/O must stage outside the session mutex'
require 'recovery\.promote\(staged\)' "$SESSION_LIFECYCLE" \
  'RemoteApp terminal publication must atomically promote the CAS-matched staged candidate'
require 'fn terminal_commit_lock' "$SESSION_STORE" \
  'RemoteApp terminal recovery promotion must serialize only the affected session'
require 'begin_terminal_commit' "$SESSION_LIFECYCLE" \
  'RemoteApp terminal publication must freeze its exact aggregate revision before recovery promotion'
require 'terminal recovery promotion' "$SESSION_LIFECYCLE" \
  'RemoteApp terminal recovery I/O must assert the global session store is unlocked'
require 'terminal_promotion_blocks_only_its_session_commit_boundary' "$SESSION_LIFECYCLE" \
  'RemoteApp terminal lifecycle must prove slow recovery I/O does not block unrelated session access'
require 'stale_staged_terminal_never_replaces_newer_closing_revision' "$SESSION_LIFECYCLE" \
  'RemoteApp terminal lifecycle must prove a stale staged terminal never becomes recovery authority'
require 'let mut terminal = session\.clone\(\);' "$SESSION_LIFECYCLE" \
  'RemoteApp terminal lifecycle must build a private terminal candidate before publication'
require '\*session = terminal;' "$SESSION_LIFECYCLE" \
  'RemoteApp terminal lifecycle must publish Closed only after durable persistence succeeds'
reject 'session\.finish_close\(' "$END_SESSION_HANDLER" \
  'RemoteApp end_session must not bypass the persistence-first terminal finalizer'
reject 'finish_permission_revoked_termination|session\.finish_close\(' "$TARGET_MONITOR" \
  'RemoteApp permission revocation must not bypass the persistence-first terminal finalizer'
reject 'session\.finish_expiration\(now\);' "$SESSION_LIFECYCLE" \
  'RemoteApp expiration must not bypass the persistence-first terminal finalizer'
require 'fn expire_session_by_id_if_needed' "$SESSION_LIFECYCLE" \
  'RemoteApp lease expiry must have a by-id orchestration boundary outside handler-held session locks'
require 'lease expiry recovery persistence' "$SESSION_LIFECYCLE" \
  'RemoteApp lease expiry recovery I/O must assert the global session store is unlocked'
require 'expire_session_by_id_if_needed\(&plugin, session_id, None\)' "$SHOW_SESSION_HANDLER" \
  'RemoteApp show_session must converge expired sessions before entering its audit read lock'
require 'end_session_commits_closed_only_after_preview_task_group_completes' "$END_SESSION_HANDLER" \
  'RemoteApp explicit close must prove Closed follows preview task-group completion'
require 'end_session_retains_closing_when_preview_completion_receipt_is_lost' "$END_SESSION_HANDLER" \
  'RemoteApp explicit close must fail closed when preview completion ownership is lost'
require 'permission_revocation_durably_stops_preview_and_clears_desired_tracking' "$TARGET_MONITOR" \
  'RemoteApp permission revocation must prove preview settlement and durable terminal cleanup'
require 'TargetSnapshotOwner::InputRequest' "$TARGET_SNAPSHOT" \
  'RemoteApp input and monitor target snapshots must share one plugin-owned native failure domain'
require 'target_local_input_provider_hang_rejects_with_bounded_deadline' "$ROOT/plugins/remote-desktop/src/input.rs" \
  'RemoteApp target-local input must prove host snapshot hangs reject within a bounded deadline'
require '50 ms monotonic deadline' "$SPEC" \
  'RemoteApp product contract must publish its bounded target-local input snapshot deadline'
require 'SESSION_REHYDRATED' "$SESSION" \
  'RemoteApp session aggregate must emit SESSION_REHYDRATED for non-terminal startup recovery'
require 'session_events::session_rehydrated' "$SESSION" \
  'RemoteApp session aggregate must emit typed SESSION_REHYDRATED projection with target binding evidence'
require 'fn rehydrate\(' "$SESSION" \
  'RemoteApp session aggregate must own snapshot-to-session rehydration'
require 'fn session_rehydrated' "$SESSION_EVENTS" \
  'RemoteApp session event projections must define typed rehydration event payloads'
require '"target_binding": binding\.to_value\(\)' "$SESSION_EVENTS" \
  'RemoteApp rehydration event payload must include selected target binding evidence'
require '"subject_ura": binding\.subject_ura\(\)' "$SESSION_EVENTS" \
  'RemoteApp rehydration event payload must bind selected Resource URA'
require 'rehydrated_non_terminal_session_can_start_new_media_epoch_without_new_session' "$SESSION" \
  'RemoteApp session aggregate must prove rehydrated sessions can restart media without minting a new session'
require 'rehydrated_non_terminal_session_preserves_runtime_input_block_reason' "$SESSION" \
  'RemoteApp session aggregate must prove non-terminal rehydrate restores runtime input blockers'
require 'recoverable_suspended_session_can_start_a_new_media_generation' "$SESSION_STATE" \
  'RemoteApp lifecycle state machine must allow rehydrated suspended sessions to restart media'
require 'recoverable_rebinding_session_can_restart_media_negotiation' "$SESSION_STATE" \
  'RemoteApp lifecycle state machine must allow recoverable rebind sessions to restart media negotiation'
for recovery_writer in \
  "$CREATE_SESSION_HANDLER" \
  "$REFRESH_LEASE_HANDLER" \
  "$SHOW_SESSION_HANDLER" \
  "$END_SESSION_HANDLER"; do
  require 'RemoteDesktopRecoverySnapshot::from_session' "$recovery_writer" \
    "RemoteApp handler must derive recovery snapshots from the session aggregate: $recovery_writer"
  require 'persist_recovery_snapshot' "$recovery_writer" \
    "RemoteApp handler must persist recovery snapshots after lifecycle mutation: $recovery_writer"
done
require 'persist_recovery_snapshot' "$SESSION_LIFECYCLE" \
  'RemoteApp lease watchdog must persist recovery snapshots for timeout transitions'
require 'recovery_store_round_trips_valid_snapshot' "$SESSION_RECOVERY" \
  'session recovery store must have snapshot round-trip coverage'
require 'recovery_store_fails_closed_for_corrupt_snapshot' "$SESSION_RECOVERY" \
  'session recovery store must fail closed for corrupt snapshots'
require 'recovery_store_load_all_reports_corrupt_snapshots_without_dropping_valid_rows' "$SESSION_RECOVERY" \
  'session recovery store must isolate corrupt snapshots during startup batch load'
require 'recovery_store_saves_private_snapshot_permissions' "$SESSION_RECOVERY" \
  'session recovery store must protect snapshots containing session tokens with private filesystem permissions'
require 'recovery_store_rejects_path_unsafe_session_ids' "$SESSION_RECOVERY" \
  'session recovery store must reject path-unsafe session ids'
require 'create_session_persists_recovery_snapshot' "$CREATE_SESSION_HANDLER" \
  'RemoteApp create_session must have regression coverage for recovery snapshot persistence'
require 'end_session_persists_terminal_recovery_snapshot' "$END_SESSION_HANDLER" \
  'RemoteApp end_session must have regression coverage for terminal recovery snapshot persistence'

require 'terminal_receipt: Option<Value>' "$SESSION" \
  'session aggregate must store a single terminal receipt projection'
require 'fn terminal_receipt\(&self\) -> Option<Value>' "$SESSION" \
  'session aggregate must expose terminal receipt to public views'
require 'project_terminal_receipt' "$SESSION" \
  'session aggregate must build terminal receipts at the lifecycle boundary'
require 'remoteapp\.session\.terminal\.v1' "$SESSION" \
  'session terminal receipt must carry a stable product receipt type'
require 'with_target_binding_context\(self\.target\.binding\(\)\)' "$SESSION" \
  'session aggregate projected-event boundary must attach selected target context'
require 'fn with_target_binding_context' "$SESSION_EVENTS" \
  'RemoteApp event projections must expose a typed target-context enrichment boundary'
require 'fn with_event_target_context' "$TARGET_TRACKING" \
  'RemoteApp target tracking state machine must own target lifecycle event context projection'
require 'to_tracking_value' "$TARGET_TRACKING" \
  'RemoteApp target tracking events must include canonical target binding evidence projected from tracker state'
require 'fn to_tracking_value' "$ROOT/plugins/remote-desktop/src/target.rs" \
  'RemoteApp target binding must expose a tracker-state projection without duplicating binding internals'
require '"scope_audit"\.to_string\(\), self\.binding\.scope_audit_value\(\)' "$TARGET_TRACKING" \
  'RemoteApp target tracking events must include capture/input scope audit evidence'
require '"latest_target_diagnostic"\.to_string\(\)' "$TARGET_TRACKING" \
  'RemoteApp target tracking events must include latest target diagnostic evidence'
require 'assert_target_tracking_payload_context' "$SESSION" \
  'RemoteApp session tests must prove target tracking events preserve full selected-target payload context'
require 'self\.terminal_receipt = Some\(self\.project_terminal_receipt' "$SESSION" \
  'session close and timeout paths must populate terminal receipt from terminal events'
require 'session_close_events_project_terminal_reason_code' "$SESSION" \
  'session tests must cover explicit-close terminal receipt projection'
require 'closing\["payload"\]\["target_binding"\]\["subject_ura"\]' "$SESSION" \
  'session close tests must prove terminal lifecycle events carry selected Resource evidence'
require 'closed\["payload"\]\["target_binding"\]\["binding_id"\]' "$SESSION" \
  'session close tests must prove closed event payload carries selected target binding'
require 'session_expiry_events_project_terminal_reason_code' "$SESSION" \
  'session tests must cover timeout terminal receipt projection'
require 'event\["payload"\]\["target_binding"\]\["subject_ura"\]' "$SESSION" \
  'session timeout tests must prove expiry terminal event carries selected Resource evidence'
require 'pub\(in crate::daemon::plugins::remote_desktop\) fn push\(' "$EVENT_LOG" \
  'event log must keep event push centralized for terminal receipt binding'
require 'event_log_push_returns_the_stored_event_record' "$EVENT_LOG" \
  'event log tests must prove push returns the stored event for terminal receipt binding'
require '"terminal_receipt": session\.terminal_receipt\(\)' "$SESSION_VIEW" \
  'session view must expose terminal_receipt instead of forcing event-log inference'
require 'session_view_projects_terminal_receipt_only_after_close' "$SESSION_VIEW" \
  'session view tests must prove terminal_receipt is null until terminal and populated after close'
require 'idempotent end_session must return the original terminal receipt' "$SESSION_HANDLERS" \
  'end_session tests must prove idempotent close returns the original terminal receipt'

printf 'check-remoteapp-product-closure-audit: ok\n'
