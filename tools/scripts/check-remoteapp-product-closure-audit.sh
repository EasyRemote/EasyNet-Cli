#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
SPEC="$ROOT/docs/design/remoteapp-targeted-session-spec.md"
AUDIT="$ROOT/docs/design/remoteapp-product-readiness-audit-2026-08-22.md"
MATRIX="$ROOT/docs/design/remoteapp-product-readiness-matrix.json"
PLAN="$ROOT/pr/20260822-remoteapp-product-closure/02-evidence-audit.md"
CROSS_DEVICE_SMOKE="$ROOT/tools/scripts/remoteapp-cross-device-product-smoke.sh"
MAIN_CRATE_IMPL_TESTS="$ROOT/tools/scripts/check-remoteapp-main-crate-implementation-tests.sh"
CAPTURE_MATRIX="$ROOT/tools/scripts/remoteapp-cross-platform-capture-e2e.sh"
INPUT_INJECTION="$ROOT/tools/scripts/remoteapp-input-injection-e2e.sh"
MEDIA_ADAPTATION="$ROOT/tools/scripts/remoteapp-media-adaptation-e2e.sh"
MULTI_WINDOW_TRACKING="$ROOT/tools/scripts/remoteapp-multi-window-tracking-e2e.sh"
NETWORK_FALLBACK="$ROOT/tools/scripts/remoteapp-network-fallback-e2e.sh"
FRONTEND_BROWSER_LIFECYCLE="$ROOT/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh"
SESSION_TIMEOUT="$ROOT/tools/scripts/host-remoteapp-session-timeout-e2e.sh"
SESSION_CANCEL="$ROOT/tools/scripts/host-remoteapp-session-cancel-e2e.sh"
PERMISSION_REVOKE="$ROOT/tools/scripts/host-remoteapp-permission-revoke-e2e.sh"
SESSION_RESUME="$ROOT/tools/scripts/host-remoteapp-session-resume-e2e.sh"
CRASH_RESTART_RECOVERY="$ROOT/tools/scripts/remoteapp-crash-restart-recovery-e2e.sh"
LIFECYCLE_HARNESS_LIB="$ROOT/tools/scripts/remoteapp-lifecycle-harness-lib.sh"
SESSION="$ROOT/plugins/remote-desktop/src/session.rs"
SESSION_EVENTS="$ROOT/plugins/remote-desktop/src/session_events.rs"
TARGET_TRACKING="$ROOT/plugins/remote-desktop/src/target_tracking.rs"
SESSION_RECOVERY="$ROOT/plugins/remote-desktop/src/session_recovery.rs"
SESSION_STATE="$ROOT/plugins/remote-desktop/src/session_state.rs"
SESSION_LIFECYCLE="$ROOT/plugins/remote-desktop/src/session_lifecycle.rs"
RUNTIME="$ROOT/plugins/remote-desktop/src/runtime.rs"
SESSION_VIEW="$ROOT/plugins/remote-desktop/src/view.rs"
SESSION_HANDLERS="$ROOT/plugins/remote-desktop/src/handlers/mod.rs"
CREATE_SESSION_HANDLER="$ROOT/plugins/remote-desktop/src/handlers/create_session.rs"
REFRESH_LEASE_HANDLER="$ROOT/plugins/remote-desktop/src/handlers/refresh_lease.rs"
SHOW_SESSION_HANDLER="$ROOT/plugins/remote-desktop/src/handlers/show_session.rs"
END_SESSION_HANDLER="$ROOT/plugins/remote-desktop/src/handlers/end_session.rs"
EVENT_LOG="$ROOT/plugins/remote-desktop/src/event_log.rs"
TARGET_MONITOR="$ROOT/plugins/remote-desktop/src/target_monitor.rs"
INPUT="$ROOT/plugins/remote-desktop/src/input.rs"

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
[[ -f "$PLAN" ]] || fail "missing RemoteApp product closure evidence plan"
[[ -f "$CROSS_DEVICE_SMOKE" ]] || fail "missing RemoteApp cross-device product smoke gate"
[[ -f "$MAIN_CRATE_IMPL_TESTS" ]] || fail "missing RemoteApp main-crate implementation test gate"
[[ -f "$CAPTURE_MATRIX" ]] || fail "missing RemoteApp cross-platform capture verifier"
[[ -f "$INPUT_INJECTION" ]] || fail "missing RemoteApp input injection verifier"
[[ -f "$MEDIA_ADAPTATION" ]] || fail "missing RemoteApp media adaptation evidence verifier"
[[ -f "$MULTI_WINDOW_TRACKING" ]] || fail "missing RemoteApp multi-window tracking evidence verifier"
[[ -f "$NETWORK_FALLBACK" ]] || fail "missing RemoteApp network fallback evidence verifier"
[[ -f "$FRONTEND_BROWSER_LIFECYCLE" ]] || fail "missing RemoteApp frontend Browser/Tauri lifecycle verifier"
[[ -f "$SESSION_TIMEOUT" ]] || fail "missing RemoteApp session timeout E2E harness"
[[ -f "$SESSION_CANCEL" ]] || fail "missing RemoteApp session cancel E2E harness"
[[ -f "$PERMISSION_REVOKE" ]] || fail "missing RemoteApp permission revoke E2E harness"
[[ -f "$SESSION_RESUME" ]] || fail "missing RemoteApp session resume E2E harness"
[[ -f "$CRASH_RESTART_RECOVERY" ]] || fail "missing RemoteApp crash/restart recovery evidence verifier"
[[ -f "$LIFECYCLE_HARNESS_LIB" ]] || fail "missing RemoteApp lifecycle harness helper library"
[[ -f "$SESSION" ]] || fail "missing RemoteApp session aggregate"
[[ -f "$TARGET_TRACKING" ]] || fail "missing RemoteApp target tracking state machine"
[[ -f "$SESSION_RECOVERY" ]] || fail "missing RemoteApp session recovery snapshot store"
[[ -f "$SESSION_STATE" ]] || fail "missing RemoteApp session lifecycle state machine"
[[ -f "$SESSION_LIFECYCLE" ]] || fail "missing RemoteApp session lifecycle module"
[[ -f "$RUNTIME" ]] || fail "missing RemoteApp runtime module"
[[ -f "$SESSION_VIEW" ]] || fail "missing RemoteApp session view projection"
[[ -f "$SESSION_HANDLERS" ]] || fail "missing RemoteApp session handler tests"
[[ -f "$CREATE_SESSION_HANDLER" ]] || fail "missing RemoteApp create_session handler"
[[ -f "$REFRESH_LEASE_HANDLER" ]] || fail "missing RemoteApp refresh_lease handler"
[[ -f "$SHOW_SESSION_HANDLER" ]] || fail "missing RemoteApp show_session handler"
[[ -f "$END_SESSION_HANDLER" ]] || fail "missing RemoteApp end_session handler"
[[ -f "$EVENT_LOG" ]] || fail "missing RemoteApp event log"
[[ -f "$TARGET_MONITOR" ]] || fail "missing RemoteApp target monitor"
[[ -f "$INPUT" ]] || fail "missing RemoteApp input execution plane"

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

require 'remoteapp_resolve_rpc_ability_ura' "$LIFECYCLE_HARNESS_LIB" \
  'RemoteApp lifecycle helper must implement catalog Ability URA resolution'
require 'remoteapp_session_approval_causal_context_json' "$LIFECYCLE_HARNESS_LIB" \
  'RemoteApp lifecycle helper must implement approval receipt causal context projection'

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
require 'Interactive app/window input must remain view-only' "$SPEC" \
  'SPEC must retain the view-only input limitation until input execution is proven'
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
require 'video-only scope' "$AUDIT" \
  'audit must record that current media pipeline support is video-only'
require 'missing media-adaptation E2E as a product blocker' "$AUDIT" \
  'audit must record missing media-adaptation E2E as a product blocker'
require 'media_pipeline_support' "$MATRIX" \
  'matrix must record media pipeline support projection evidence'
require 'media_pipeline_support' "$PLAN" \
  'plan evidence audit must record media pipeline support projection evidence'
require 'Linux display is diagnostic-only' "$AUDIT" \
  'audit must record Linux display diagnostic-only support state'
require 'Windows display/window/application are unsupported' "$AUDIT" \
  'audit must record Windows unsupported capture state'
require 'Linux/Windows input injection is unsupported' "$AUDIT" \
  'audit must record Linux/Windows unsupported input state'
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
require 'Linux app/window and Windows capture explicitly unsupported' "$MATRIX" \
  'product readiness matrix must record explicit Linux/Windows unsupported capture state'
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
require 'real_media_adaptation_matrix' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must require real media adaptation proof mode'
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
require 'audio.codec must match across media scenarios' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must compare one audio codec across scenarios'
require 'audio.status must be passed' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must require live host audio evidence'
require 'host audio unsupported state is not product media evidence' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must reject host-audio unsupported evidence'
require 'queue.observed_max_depth must not exceed max_depth' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must reject unbounded queue evidence'
require 'remote_desktop\.create_session' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must inspect create_session evidence'
require 'remote_desktop\.attach' "$MEDIA_ADAPTATION" \
  'media adaptation verifier must inspect attach evidence'
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
require 'webrtc caller_device_ura must bind caller device' "$NETWORK_FALLBACK" \
  'network fallback verifier must bind WebRTC evidence to caller device'
require 'webrtc callee_device_ura must bind callee device' "$NETWORK_FALLBACK" \
  'network fallback verifier must bind WebRTC evidence to callee device'
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
require 'local_candidate_type' "$ROOT/plugins/remote-desktop/src/media/native.rs" \
  'native WebRTC stats must project selected local candidate type for network fallback evidence'
require 'remote_candidate_type' "$ROOT/plugins/remote-desktop/src/media/native.rs" \
  'native WebRTC stats must project selected remote candidate type for network fallback evidence'
require 'selected_route_class' "$ROOT/plugins/remote-desktop/src/media/native.rs" \
  'native WebRTC stats must project selected route class for network fallback evidence'
require '"protocol"' "$ROOT/plugins/remote-desktop/src/media/native.rs" \
  'native WebRTC stats must project selected candidate pair protocol for network fallback evidence'
require 'remote_desktop\.create_session' "$NETWORK_FALLBACK" \
  'network fallback verifier must inspect create_session evidence'
require 'remote_desktop\.attach' "$NETWORK_FALLBACK" \
  'network fallback verifier must inspect attach evidence'
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
require 'real_browser_tauri_lifecycle' "$FRONTEND_BROWSER_LIFECYCLE" \
  'frontend Browser/Tauri lifecycle verifier must require real lifecycle proof mode'
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
require 'remote_desktop\.attach' "$FRONTEND_BROWSER_LIFECYCLE" \
  'frontend Browser/Tauri lifecycle verifier must inspect attach evidence'
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
require 'terminal receipt id must be replayed' "$CRASH_RESTART_RECOVERY" \
  'crash/restart recovery verifier must require original terminal receipt replay'
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
require 'harden_recovery_file' "$SESSION_RECOVERY" \
  'session recovery store must harden daemon-local recovery files because snapshots include session tokens'
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
require 'desired: Mutex<HashSet<String>>' "$TARGET_MONITOR" \
  'RemoteApp target monitor must keep plugin-owned desired tracking state outside the worker thread'
require 'initial_tracked: HashSet<String>' "$TARGET_MONITOR" \
  'RemoteApp target monitor worker restarts must be seeded from desired tracking state'
require 'desired_sessions_for_test' "$TARGET_MONITOR" \
  'RemoteApp target monitor must expose test evidence for desired tracking state'
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
