#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
SPEC="$ROOT/docs/design/remoteapp-targeted-session-spec.md"
AUDIT="$ROOT/docs/design/remoteapp-product-readiness-audit-2026-08-22.md"
MATRIX="$ROOT/docs/design/remoteapp-product-readiness-matrix.json"
PLAN="$ROOT/pr/20260822-remoteapp-product-closure/02-evidence-audit.md"
CROSS_DEVICE_SMOKE="$ROOT/tools/scripts/remoteapp-cross-device-product-smoke.sh"
CAPTURE_MATRIX="$ROOT/tools/scripts/remoteapp-cross-platform-capture-e2e.sh"
NETWORK_FALLBACK="$ROOT/tools/scripts/remoteapp-network-fallback-e2e.sh"
FRONTEND_BROWSER_LIFECYCLE="$ROOT/tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh"
SESSION_TIMEOUT="$ROOT/tools/scripts/host-remoteapp-session-timeout-e2e.sh"
SESSION_CANCEL="$ROOT/tools/scripts/host-remoteapp-session-cancel-e2e.sh"
PERMISSION_REVOKE="$ROOT/tools/scripts/host-remoteapp-permission-revoke-e2e.sh"
SESSION_RESUME="$ROOT/tools/scripts/host-remoteapp-session-resume-e2e.sh"
SESSION="$ROOT/plugins/remote-desktop/src/session.rs"
SESSION_VIEW="$ROOT/plugins/remote-desktop/src/view.rs"
SESSION_HANDLERS="$ROOT/plugins/remote-desktop/src/handlers/mod.rs"
EVENT_LOG="$ROOT/plugins/remote-desktop/src/event_log.rs"

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
[[ -f "$CAPTURE_MATRIX" ]] || fail "missing RemoteApp cross-platform capture verifier"
[[ -f "$NETWORK_FALLBACK" ]] || fail "missing RemoteApp network fallback evidence verifier"
[[ -f "$FRONTEND_BROWSER_LIFECYCLE" ]] || fail "missing RemoteApp frontend Browser/Tauri lifecycle verifier"
[[ -f "$SESSION_TIMEOUT" ]] || fail "missing RemoteApp session timeout E2E harness"
[[ -f "$SESSION_CANCEL" ]] || fail "missing RemoteApp session cancel E2E harness"
[[ -f "$PERMISSION_REVOKE" ]] || fail "missing RemoteApp permission revoke E2E harness"
[[ -f "$SESSION_RESUME" ]] || fail "missing RemoteApp session resume E2E harness"
[[ -f "$SESSION" ]] || fail "missing RemoteApp session aggregate"
[[ -f "$SESSION_VIEW" ]] || fail "missing RemoteApp session view projection"
[[ -f "$SESSION_HANDLERS" ]] || fail "missing RemoteApp session handler tests"
[[ -f "$EVENT_LOG" ]] || fail "missing RemoteApp event log"

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
require 'Audio/video codec, frame rate, bitrate adaptation' "$AUDIT" \
  'audit must cover media codec/adaptation'
require 'Multi-window/multi-application independent tracking' "$AUDIT" \
  'audit must cover multi-window/application tracking as execution effect'
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
require 'Cross-device E2E smoke/regression exists beyond local provider boundary' "$AUDIT" \
  'audit must cover cross-device proof'
require 'remoteapp-cross-device-product-smoke.sh' "$AUDIT" \
  'audit must name the cross-device product smoke gate'
require 'governed Hub routing, cross-device ability visibility/invocation' "$AUDIT" \
  'audit must scope cross-device smoke to routing and synthetic media evidence'
require 'does not prove real' "$AUDIT" \
  'audit must reject cross-device smoke as real OS capture proof'
require 'accepted_count=0, expected_count=5' "$AUDIT" \
  'audit must record the current cross-device service owner projection failure'
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
require 'Cross-platform capture implementation/evidence using' "$PLAN" \
  'plan evidence audit must list missing Windows/Linux evidence'
require 'remoteapp-cross-platform-capture-e2e\.sh' "$PLAN" \
  'plan evidence audit must record the cross-platform capture verifier'
require 'Windows/Linux capture or explicit product unsupported state' "$PLAN" \
  'plan evidence audit must preserve Windows/Linux capture or unsupported requirement'
require 'Frontend full lifecycle E2E' "$PLAN" \
  'plan evidence audit must list frontend full lifecycle E2E as missing'
require 'frontend-remoteapp-browser-lifecycle-e2e\.sh' "$PLAN" \
  'plan evidence audit must record the Browser/Tauri lifecycle verifier'
require 'remoteapp-cross-device-product-smoke.sh' "$PLAN" \
  'plan evidence audit must record the cross-device smoke gate'
require 'failed at `cross-device-routing`' "$PLAN" \
  'plan evidence audit must record the latest cross-device smoke failure'
require 'accepted_count=0, expected_count=5' "$PLAN" \
  'plan evidence audit must preserve the service owner projection failure evidence'
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
  'cross-device smoke must classify current Service owner projection failures'
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
require 'terminal_receipt' "$CAPTURE_MATRIX" \
  'cross-platform capture verifier must inspect terminal receipt evidence'
require 'product_complete_claim.*False|product_complete_claim.*false' "$CAPTURE_MATRIX" \
  'cross-platform capture verifier must reject product completion claims'
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
require 'remote_desktop\.permission_status' "$FRONTEND_BROWSER_LIFECYCLE" \
  'frontend Browser/Tauri lifecycle verifier must inspect permission_status evidence'
require 'remote_desktop\.create_session' "$FRONTEND_BROWSER_LIFECYCLE" \
  'frontend Browser/Tauri lifecycle verifier must inspect create_session evidence'
require 'remote_desktop\.attach' "$FRONTEND_BROWSER_LIFECYCLE" \
  'frontend Browser/Tauri lifecycle verifier must inspect attach evidence'
require 'remote_desktop\.watch_events' "$FRONTEND_BROWSER_LIFECYCLE" \
  'frontend Browser/Tauri lifecycle verifier must inspect watch_events evidence'
require 'remote_desktop\.end_session' "$FRONTEND_BROWSER_LIFECYCLE" \
  'frontend Browser/Tauri lifecycle verifier must inspect end_session evidence'
require 'terminal_receipt_visible' "$FRONTEND_BROWSER_LIFECYCLE" \
  'frontend Browser/Tauri lifecycle verifier must inspect terminal receipt visibility'
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

require 'terminal_receipt: Option<Value>' "$SESSION" \
  'session aggregate must store a single terminal receipt projection'
require 'fn terminal_receipt\(&self\) -> Option<Value>' "$SESSION" \
  'session aggregate must expose terminal receipt to public views'
require 'project_terminal_receipt' "$SESSION" \
  'session aggregate must build terminal receipts at the lifecycle boundary'
require 'remoteapp\.session\.terminal\.v1' "$SESSION" \
  'session terminal receipt must carry a stable product receipt type'
require 'self\.terminal_receipt = Some\(self\.project_terminal_receipt' "$SESSION" \
  'session close and timeout paths must populate terminal receipt from terminal events'
require 'session_close_events_project_terminal_reason_code' "$SESSION" \
  'session tests must cover explicit-close terminal receipt projection'
require 'session_expiry_events_project_terminal_reason_code' "$SESSION" \
  'session tests must cover timeout terminal receipt projection'
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
