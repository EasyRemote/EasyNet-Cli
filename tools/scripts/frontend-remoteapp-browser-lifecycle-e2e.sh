#!/usr/bin/env bash
# Frontend RemoteApp Browser/Tauri lifecycle E2E evidence verifier.
#
# Boundary:
# - This harness verifies evidence produced by a real browser/Tauri runner for
#   the frontend RemoteApp lifecycle. It does not replace daemon/host E2E
#   harnesses and does not simulate browser UI actions.
# - A live pass requires either --evidence-json from an external runner or
#   --runner-cmd that writes the evidence JSON path provided through
#   EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_EVIDENCE_JSON.
# - Self-test validates the evidence contract only; it is not product evidence.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"

MODE=skip
SELF_TEST=0
OUT_DIR="${EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_OUT_DIR:-$REPO_ROOT/target/e2e/frontend-remoteapp-browser-lifecycle/$(date -u +%Y%m%d-%H%M%S)-$$}"
FRONTEND_URL="${EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_FRONTEND_URL:-}"
SURFACE="${EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_SURFACE:-browser}"
RUNNER_CMD="${EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_RUNNER_CMD:-}"
EVIDENCE_INPUT="${EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_EVIDENCE_JSON:-}"

usage() {
  cat <<'USAGE'
Usage:
  frontend-remoteapp-browser-lifecycle-e2e.sh --run --evidence-json PATH
  frontend-remoteapp-browser-lifecycle-e2e.sh --run --runner-cmd CMD --frontend-url URL
  frontend-remoteapp-browser-lifecycle-e2e.sh --self-test

Options:
  --run                 Verify real Browser/Tauri lifecycle evidence.
  --self-test           Validate the harness against synthetic positive evidence.
  --frontend-url URL    Browser/Tauri app URL used by the external runner.
  --surface KIND        browser or tauri. Default: browser.
  --runner-cmd CMD      Command that drives the real UI and writes evidence to
                        EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_EVIDENCE_JSON.
  --evidence-json PATH  Existing evidence JSON emitted by a real UI runner.
  --out-dir DIR         Report directory.
  -h, --help            Show this help.

Environment:
  EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_E2E=1
                        Equivalent to --run.

Evidence contract:
  The evidence JSON must prove a real Browser/Tauri flow, not component mocks:
  app_loaded -> authenticated_session -> target_picker_opened ->
  permission_status_checked -> consent_granted -> session_created ->
  webrtc_transport_connected -> watch_events_streaming -> media_presented ->
  media_pipeline_support_visible -> input_control_attempted_or_policy_blocked
  -> session_ended -> terminal_receipt_visible. The WebRTC step must show a
  connected peer path, the media step must show a visible rendered media element
  with positive frame count, every step must carry real browser/Tauri automation
  evidence with monotonic observation timestamps, and the input step must show
  either applied input telemetry or an explicit policy block. If the input step
  claims input_applied, it must include the submitted data-channel frame and the
  daemon applied event with matching client_sequence and target_focus_epoch.
  Product transport-resume evidence additionally requires:
  transport_disconnected -> session_preserved_for_reconnect ->
  transport_reconnected -> watch_events_reestablished ->
  media_presented_after_resume -> input_control_after_resume. The runner must
  prove a retired old PeerConnection, a new daemon-issued transport epoch, a
  new PeerConnection, decoded frames, and unchanged input authority.

Runner environment for the bundled browser runner:
  EASYNET_REMOTEAPP_BROWSER_RESUME_DISCONNECT_COMMAND
  EASYNET_REMOTEAPP_BROWSER_RESUME_RECONNECT_COMMAND
                        Both must be set to exercise real transport resume.
                        The disconnect command must make the selected device
                        offline from the Hub/frontend perspective; the reconnect
                        command must restore that same device Runtime.

Non-claims:
  A skipped report or self-test does not prove frontend product readiness.
  Without the paired resume commands, this harness does not emit a transport
  resume summary. Cross-device, OS input injection, codec soak, and network
  fallback still require their own evidence.
USAGE
}

if [[ "${EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_E2E:-0}" == "1" ]]; then
  MODE=run
fi

while [[ $# -gt 0 ]]; do
  case "$1" in
    --run) MODE=run; shift ;;
    --self-test) SELF_TEST=1; MODE=self-test; shift ;;
    --frontend-url) FRONTEND_URL="${2:?missing value for --frontend-url}"; shift 2 ;;
    --surface)
      case "${2:?missing value for --surface}" in
        browser|tauri) SURFACE="$2" ;;
        *) echo "invalid surface: $2" >&2; exit 64 ;;
      esac
      shift 2
      ;;
    --runner-cmd) RUNNER_CMD="${2:?missing value for --runner-cmd}"; shift 2 ;;
    --evidence-json) EVIDENCE_INPUT="${2:?missing value for --evidence-json}"; shift 2 ;;
    --out-dir) OUT_DIR="${2:?missing value for --out-dir}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

mkdir -p "$OUT_DIR"
EVIDENCE_JSON="$OUT_DIR/evidence.json"
REPORT_JSON="$OUT_DIR/report.json"
REPORT_MD="$OUT_DIR/report.md"
RUNNER_STDOUT="$OUT_DIR/runner.stdout.txt"
RUNNER_STDERR="$OUT_DIR/runner.stderr.txt"

write_report() {
  local status="$1"
  local reason="$2"
  python3 - "$REPORT_JSON" "$REPORT_MD" "$status" "$reason" "$SURFACE" "$FRONTEND_URL" "$EVIDENCE_JSON" <<'PY'
import json
import pathlib
import sys

report_path, md_path, status, reason, surface, frontend_url, evidence_path = sys.argv[1:8]
report = {
    "script": "tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh",
    "status": status,
    "reason": reason,
    "surface": surface,
    "frontend_url": frontend_url,
    "evidence_json": evidence_path,
    "product_complete_claim": False,
}
pathlib.Path(report_path).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
pathlib.Path(md_path).write_text(
    "# Frontend RemoteApp Browser/Tauri Lifecycle E2E\n\n"
    f"- Status: `{status}`\n"
    f"- Surface: `{surface}`\n"
    f"- Frontend URL: `{frontend_url}`\n"
    f"- Reason: `{reason}`\n"
    f"- Evidence: `{evidence_path}`\n",
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

def int_field(obj, key, default=0):
    if not isinstance(obj, dict):
        return default
    try:
        return int(obj.get(key, default))
    except (TypeError, ValueError):
        return default

required_steps = [
    "app_loaded",
    "authenticated_session",
    "target_picker_opened",
    "permission_status_checked",
    "consent_granted",
    "session_created",
    "webrtc_transport_connected",
    "watch_events_streaming",
    "media_presented",
    "media_pipeline_support_visible",
    "input_control_attempted_or_policy_blocked",
    "session_ended",
    "terminal_receipt_visible",
]
ability_steps = {
    "permission_status_checked": "remote_desktop.permission_status",
    "consent_granted": "remote_desktop.grant_consent",
    "session_created": "remote_desktop.create_session",
    "webrtc_transport_connected": "remote_desktop.set_description",
    "watch_events_streaming": "remote_desktop.watch_events",
    "session_ended": "remote_desktop.end_session",
}

require(evidence.get("status") == "passed", "evidence.status must be passed")
require(evidence.get("proof_mode") == "real_browser_tauri_lifecycle",
        "proof_mode must be real_browser_tauri_lifecycle")
require(evidence.get("runner_kind") in {"browser", "tauri"},
        "runner_kind must be browser or tauri")
require(evidence.get("component_mock") is False,
        "component_mock must be false")
require(evidence.get("real_backend_runtime") is True,
        "real_backend_runtime must be true")
require(evidence.get("product_complete_claim") is False,
        "product_complete_claim must remain false")
require(isinstance(evidence.get("frontend_url"), str)
        and evidence["frontend_url"].startswith(("http://", "https://", "tauri://")),
        "frontend_url must identify the real frontend surface")

device_ura = evidence.get("device_ura")
subject_ura = evidence.get("selected_resource_ura")
session_id = evidence.get("session_id")
require(isinstance(device_ura, str) and device_ura.startswith("easynet:///"),
        "device_ura must be a canonical EasyNet URA")
require(isinstance(subject_ura, str) and subject_ura.startswith("easynet:///"),
        "selected_resource_ura must be a canonical EasyNet Resource URA")
require(isinstance(session_id, str) and session_id,
        "session_id must be recorded")

steps = evidence.get("steps")
require(isinstance(steps, list) and steps, "steps must be a non-empty list")
step_names = []
step_by_name = {}
last_observed_at_ms = 0
if isinstance(steps, list):
    for step in steps:
        if not isinstance(step, dict):
            errors.append("each step must be an object")
            continue
        name = step.get("name")
        if not isinstance(name, str):
            errors.append("each step must have a name")
            continue
        step_names.append(name)
        step_by_name[name] = step
        require(step.get("status") == "passed", f"{name}: status must be passed")
        require(step.get("evidence_source") in {"browser_automation", "tauri_automation"},
                f"{name}: evidence_source must be browser_automation or tauri_automation")
        require(step.get("component_snapshot_only") is not True,
                f"{name}: component_snapshot_only must not be true")
        try:
            observed_at_ms = int(step.get("observed_at_ms", 0))
        except (TypeError, ValueError):
            observed_at_ms = 0
        require(observed_at_ms > 0, f"{name}: observed_at_ms must be positive")
        require(observed_at_ms > last_observed_at_ms,
                f"{name}: observed_at_ms must be strictly increasing")
        last_observed_at_ms = observed_at_ms

cursor = -1
for required in required_steps:
    try:
        index = step_names.index(required)
    except ValueError:
        errors.append(f"missing lifecycle step: {required}")
        continue
    require(index > cursor, f"lifecycle step order is wrong at {required}")
    cursor = index

for step_name, ability in ability_steps.items():
    step = step_by_name.get(step_name)
    if not isinstance(step, dict):
        continue
    require(step.get("ability") == ability, f"{step_name}: ability must be {ability}")
    if step_name != "permission_status_checked":
        require(step.get("subject_ura") == subject_ura,
                f"{step_name}: subject_ura must equal selected Resource URA")
    else:
        require(step.get("subject_ura") in {None, ""},
                "permission_status_checked must be host-local and not target-scoped")

created = step_by_name.get("session_created", {})
ended = step_by_name.get("session_ended", {})
terminal = step_by_name.get("terminal_receipt_visible", {})
watch = step_by_name.get("watch_events_streaming", {})
attached = step_by_name.get("webrtc_transport_connected", {})
media = step_by_name.get("media_presented", {})
pipeline = step_by_name.get("media_pipeline_support_visible", {})
input_step = step_by_name.get("input_control_attempted_or_policy_blocked", {})
require(created.get("session_id") == session_id,
        "session_created must bind the top-level session_id")
require(attached.get("session_id") == session_id,
        "webrtc_transport_connected must bind the created session_id")
require(attached.get("rtc_connection_state") in {"connected", "completed"},
        "webrtc_transport_connected.rtc_connection_state must be connected or completed")
require(attached.get("ice_connection_state") in {"connected", "completed"},
        "webrtc_transport_connected.ice_connection_state must be connected or completed")
require(attached.get("media_stream_attached") is True,
        "webrtc_transport_connected must prove media_stream_attached=true")
require(watch.get("session_id") == session_id,
        "watch_events_streaming must bind the created session_id")
require(ended.get("session_id") == session_id,
        "session_ended must bind the created session_id")
require(media.get("frame_presented") is True,
        "media_presented must prove at least one rendered media frame")
require(media.get("media_element_visible") is True,
        "media_presented must prove media_element_visible=true")
try:
    frames_presented = int(media.get("frames_presented", 0))
except (TypeError, ValueError):
    frames_presented = 0
require(frames_presented > 0,
        "media_presented.frames_presented must be positive")
pipeline_label = pipeline.get("visible_label")
require(isinstance(pipeline_label, str) and pipeline_label,
        "media_pipeline_support_visible must include visible_label")
if isinstance(pipeline_label, str):
    for token in (
        "pipeline ",
        "h264",
        "bounded_queue_drop_stale_frames",
    ):
        require(token in pipeline_label,
                f"media_pipeline_support_visible.visible_label must include {token}")
media_scope = pipeline.get("media_scope")
require(media_scope in {"video_only", "audio_video"},
        "media_pipeline_support_visible.media_scope must be video_only or audio_video")
require(pipeline.get("product_ready") is False,
        "media_pipeline_support_visible must keep product_ready=false")
selected_route_class = pipeline.get("selected_route_class")
selected_pair_state = pipeline.get("selected_pair_state")
selected_pair_nominated = pipeline.get("selected_pair_nominated")
selected_local_candidate_type = pipeline.get("selected_local_candidate_type")
selected_remote_candidate_type = pipeline.get("selected_remote_candidate_type")
selected_candidate_protocol = pipeline.get("selected_candidate_protocol")
require(selected_route_class in {"direct", "stun_srflx", "relay"},
        "media_pipeline_support_visible must expose a known selected_route_class")
require(isinstance(selected_pair_state, str)
        and selected_pair_state.lower() == "succeeded",
        "media_pipeline_support_visible must expose a succeeded selected pair")
require(selected_pair_nominated is True,
        "media_pipeline_support_visible must prove the selected pair is nominated")
require(selected_local_candidate_type in {"host", "srflx", "prflx", "relay"},
        "media_pipeline_support_visible must expose the selected local candidate type")
require(selected_remote_candidate_type in {"host", "srflx", "prflx", "relay"},
        "media_pipeline_support_visible must expose the selected remote candidate type")
require(selected_candidate_protocol in {"udp", "tcp"},
        "media_pipeline_support_visible must expose the selected candidate protocol")
if selected_route_class == "direct":
    require("host" in {selected_local_candidate_type, selected_remote_candidate_type},
            "direct selected route must include a host candidate")
if selected_route_class == "stun_srflx":
    require(bool({selected_local_candidate_type, selected_remote_candidate_type} & {"srflx", "prflx"}),
            "stun_srflx selected route must include a reflexive candidate")
if selected_route_class == "relay":
    require("relay" in {selected_local_candidate_type, selected_remote_candidate_type},
            "relay selected route must include a relay candidate")
blockers = pipeline.get("product_blockers")
require(isinstance(blockers, list)
        and "remoteapp_media_adaptation_e2e_artifact_missing" in blockers,
        "media_pipeline_support_visible must expose the live media-adaptation E2E blocker")
if media_scope == "video_only":
    require(isinstance(blockers, list) and "host_audio_not_implemented" in blockers,
            "video_only media must expose host_audio_not_implemented")
if media_scope == "audio_video":
    require(attached.get("audio_track_attached") is True,
            "audio_video media must prove an attached WebRTC audio track")
require(input_step.get("result") in {"input_applied", "policy_blocked"},
        "input control must either apply input or prove policy_blocked")
input_status = input_step.get("visible_status")
require(isinstance(input_status, str) and input_status,
        "input control step must expose visible_status")
if input_step.get("result") == "policy_blocked":
    blocked_reason = input_step.get("blocked_reason")
    require(blocked_reason in {
        "view_only",
        "input_scope_unsupported",
        "input_permission_blocked",
        "target_input_not_ready",
        "target_blurred",
        "target_focus_unobserved",
        "target_hidden",
        "target_minimized",
        "target_lost",
        "target_stale",
        "target_unresolved",
        "target_rebinding",
        "target_invalidated",
        "accessibility_permission_denied",
        "windows_send_input_denied",
        "linux_xtest_injection_denied",
        "platform_input_injection_unavailable",
        "input_injection_unavailable",
        "input_control_consent_missing",
    }, "policy_blocked input must expose a known blocked_reason")
    target_tracking = input_step.get("target_tracking")
    require(isinstance(target_tracking, dict),
            "policy_blocked input must include target_tracking evidence")
    if isinstance(target_tracking, dict):
        require(isinstance(target_tracking.get("status"), str) and target_tracking["status"],
                "policy_blocked target_tracking.status must be present")
        require(isinstance(target_tracking.get("visibility"), str) and target_tracking["visibility"],
                "policy_blocked target_tracking.visibility must be present")
        require(isinstance(target_tracking.get("input_enabled"), bool),
                "policy_blocked target_tracking.input_enabled must be boolean")
        if evidence.get("selected_target_kind") in {"window", "application"}:
            require(int_field(target_tracking, "focus_epoch") > 0,
                    "window/application policy block must include a positive target focus epoch")
            require(int_field(target_tracking, "geometry_revision") > 0,
                    "window/application policy block must include a positive target geometry revision")
        if isinstance(blocked_reason, str) and blocked_reason.startswith("target_"):
            require(target_tracking.get("input_enabled") is False,
                    "target lifecycle policy block must prove target input_enabled=false")
            require(target_tracking.get("input_blocked_reason") == blocked_reason,
                    "target lifecycle blocker must match target_tracking.input_blocked_reason")
if input_step.get("result") == "input_applied":
    client_sequence = int_field(input_step, "client_sequence")
    target_focus_epoch = int_field(input_step, "target_focus_epoch")
    submitted_frame = input_step.get("submitted_frame")
    applied_event = input_step.get("applied_event")
    latency_ms = -1
    try:
        latency_ms = float(input_step.get("latency_ms", -1))
    except (TypeError, ValueError):
        pass
    require(client_sequence > 0,
            "input_applied must include positive client_sequence")
    require(target_focus_epoch > 0,
            "input_applied target_focus_epoch must be positive")
    require(0 <= latency_ms <= 250,
            "input_applied latency_ms must be within frontend lifecycle bound")
    require(isinstance(submitted_frame, dict),
            "input_applied must include submitted_frame")
    if isinstance(submitted_frame, dict):
        require(submitted_frame.get("type") in {"pointer", "wheel", "key", "keyboard"},
                "submitted_frame must be a RemoteApp pointer/wheel/key frame")
        require(int_field(submitted_frame, "client_sequence") == client_sequence,
                "submitted_frame client_sequence must match input_applied client_sequence")
        require(int_field(submitted_frame, "sent_at_ms") > 0,
                "submitted_frame sent_at_ms must be positive")
        require(int_field(submitted_frame, "target_focus_epoch") == target_focus_epoch,
                "submitted_frame target_focus_epoch must match input_applied target_focus_epoch")
        target_geometry_revision = input_step.get("target_geometry_revision")
        if target_geometry_revision is not None:
            require(int_field(submitted_frame, "target_geometry_revision") == int_field(input_step, "target_geometry_revision"),
                    "submitted_frame target_geometry_revision must match input_applied target_geometry_revision")
    require(isinstance(applied_event, dict),
            "input_applied must include daemon applied_event")
    if isinstance(applied_event, dict):
        require(applied_event.get("event_type") in {"INPUT_FRAME_APPLIED", "input_frame_applied"},
                "applied_event.event_type must prove INPUT_FRAME_APPLIED")
        require(applied_event.get("session_id") == session_id,
                "applied_event session_id must bind the created session id")
        require(int_field(applied_event, "client_sequence") == client_sequence,
                "applied_event client_sequence must match input_applied client_sequence")
        require(int_field(applied_event, "target_focus_epoch") == target_focus_epoch,
                "applied_event target_focus_epoch must match input_applied target_focus_epoch")
        if input_step.get("target_geometry_revision") is not None:
            require(int_field(applied_event, "target_geometry_revision") == int_field(input_step, "target_geometry_revision"),
                    "applied_event target_geometry_revision must match input_applied target_geometry_revision")

transport_resume = evidence.get("transport_resume")
if transport_resume is not None:
    require(isinstance(transport_resume, dict),
            "transport_resume must be an object when present")
    if isinstance(transport_resume, dict):
        resume_steps = [
            "transport_disconnected",
            "session_preserved_for_reconnect",
            "transport_reconnected",
            "watch_events_reestablished",
            "media_presented_after_resume",
            "input_control_after_resume",
        ]
        resume_cursor = step_names.index("input_control_attempted_or_policy_blocked")
        for resume_step in resume_steps:
            try:
                resume_index = step_names.index(resume_step)
            except ValueError:
                errors.append(f"missing transport resume step: {resume_step}")
                continue
            require(resume_index > resume_cursor,
                    f"transport resume step order is wrong at {resume_step}")
            resume_cursor = resume_index

        prior_epoch = int_field(transport_resume, "prior_transport_epoch")
        resumed_epoch = int_field(transport_resume, "transport_epoch")
        require(transport_resume.get("proof_mode") == "real_browser_transport_resume",
                "transport_resume.proof_mode must be real_browser_transport_resume")
        require(transport_resume.get("session_id") == session_id,
                "transport_resume must preserve the created session id")
        require(transport_resume.get("subject_ura") == subject_ura,
                "transport_resume must remain bound to the selected Resource URA")
        for field in (
            "same_public_session",
            "old_peer_retired",
            "new_peer_connection",
            "transport_epoch_increased",
            "watch_events_reestablished",
            "input_authority_preserved",
        ):
            require(transport_resume.get(field) is True,
                    f"transport_resume.{field} must be true")
        require(prior_epoch > 0, "transport_resume.prior_transport_epoch must be positive")
        require(resumed_epoch > prior_epoch,
                "transport_resume.transport_epoch must exceed prior_transport_epoch")
        require(int_field(transport_resume, "frames_presented_after_resume") > 0,
                "transport_resume.frames_presented_after_resume must be positive")
        require(transport_resume.get("input_result_before") in {"input_applied", "policy_blocked"},
                "transport_resume.input_result_before must be explicit")
        require(transport_resume.get("input_result_before") == input_step.get("result"),
                "transport_resume.input_result_before must match initial input evidence")
        require(transport_resume.get("input_result_after") == transport_resume.get("input_result_before"),
                "transport_resume must preserve input authority across reconnect")

        disconnected_step = step_by_name.get("transport_disconnected", {})
        preserved_step = step_by_name.get("session_preserved_for_reconnect", {})
        reconnected_step = step_by_name.get("transport_reconnected", {})
        watch_resume_step = step_by_name.get("watch_events_reestablished", {})
        media_resume_step = step_by_name.get("media_presented_after_resume", {})
        input_resume_step = step_by_name.get("input_control_after_resume", {})
        require(disconnected_step.get("old_peer_retired") is True,
                "transport_disconnected must prove the old PeerConnection retired")
        require(disconnected_step.get("transport_epoch") == prior_epoch,
                "transport_disconnected must bind the prior transport epoch")
        require(preserved_step.get("same_session") is True and preserved_step.get("terminal") is False,
                "session_preserved_for_reconnect must prove the same non-terminal session")
        require(reconnected_step.get("same_session") is True,
                "transport_reconnected must preserve the public session")
        require(reconnected_step.get("new_peer_connection") is True,
                "transport_reconnected must prove a new PeerConnection")
        require(int_field(reconnected_step, "prior_transport_epoch") == prior_epoch,
                "transport_reconnected prior epoch must match the resume summary")
        require(int_field(reconnected_step, "transport_epoch") == resumed_epoch,
                "transport_reconnected epoch must match the resume summary")
        require(reconnected_step.get("transport_epoch_increased") is True,
                "transport_reconnected must prove transport epoch advancement")
        require(reconnected_step.get("rtc_connection_state") == "connected",
                "transport_reconnected RTC state must be connected")
        require(reconnected_step.get("ice_connection_state") in {"connected", "completed"},
                "transport_reconnected ICE state must be connected or completed")
        require(int_field(watch_resume_step, "subscription_count")
                > int_field(watch_resume_step, "prior_subscription_count"),
                "watch_events_reestablished must prove a new subscription")
        require(int_field(media_resume_step, "transport_epoch") == resumed_epoch,
                "media_presented_after_resume must bind the new transport epoch")
        require(int_field(media_resume_step, "frames_presented") > 0,
                "media_presented_after_resume must prove decoded frames")
        require(input_resume_step.get("result") == transport_resume.get("input_result_after"),
                "input_control_after_resume must match the resume input summary")
        if input_resume_step.get("result") == "input_applied":
            resumed_sequence = int_field(input_resume_step, "client_sequence")
            require(resumed_sequence > int_field(input_step, "client_sequence"),
                    "post-resume input client_sequence must advance")
            resumed_submitted = input_resume_step.get("submitted_frame")
            resumed_applied = input_resume_step.get("applied_event")
            require(isinstance(resumed_submitted, dict),
                    "post-resume input must include submitted_frame")
            require(isinstance(resumed_applied, dict),
                    "post-resume input must include applied_event")
            if isinstance(resumed_submitted, dict):
                require(int_field(resumed_submitted, "client_sequence") == resumed_sequence,
                        "post-resume submitted_frame sequence must match")
                require(int_field(resumed_submitted, "target_focus_epoch")
                        == int_field(input_resume_step, "target_focus_epoch"),
                        "post-resume submitted_frame focus epoch must match")
            if isinstance(resumed_applied, dict):
                require(resumed_applied.get("session_id") == session_id,
                        "post-resume applied_event must preserve the session id")
                require(int_field(resumed_applied, "client_sequence") == resumed_sequence,
                        "post-resume applied_event sequence must match")
                require(int_field(resumed_applied, "target_focus_epoch")
                        == int_field(input_resume_step, "target_focus_epoch"),
                        "post-resume applied_event focus epoch must match")
        if input_resume_step.get("result") == "policy_blocked":
            require(input_resume_step.get("blocked_reason") == input_step.get("blocked_reason"),
                    "post-resume input policy block must remain stable")

        snapshots = evidence.get("transport_snapshots")
        require(isinstance(snapshots, list),
                "transport resume evidence must include transport_snapshots")
        if isinstance(snapshots, list):
            matching_epochs = {
                int_field(snapshot, "transport_epoch")
                for snapshot in snapshots
                if isinstance(snapshot, dict)
                and snapshot.get("session_id") == session_id
                and snapshot.get("subject_ura") == subject_ura
            }
            require(prior_epoch in matching_epochs and resumed_epoch in matching_epochs,
                    "transport_snapshots must contain both daemon-issued epochs")
require(terminal.get("reason_code") in {"user_cancelled", "caller_ended", "resume_e2e_cleanup"},
        "terminal_receipt_visible must expose a known end reason")
require(terminal.get("terminal") is True,
        "terminal_receipt_visible must expose terminal=true")
require(terminal.get("session_id") == session_id,
        "terminal receipt must bind the created session id")

report = {
    "script": "tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh",
    "status": "failed" if errors else "passed",
    "errors": errors,
    "surface": evidence.get("runner_kind"),
    "frontend_url": evidence.get("frontend_url"),
    "session_id": session_id,
    "selected_resource_ura": subject_ura,
    "evidence_json": evidence_path,
    **({"transport_resume_summary": transport_resume}
       if isinstance(transport_resume, dict) else {}),
    "product_complete_claim": False,
}
pathlib.Path(report_path).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
with open(md_path, "w", encoding="utf-8") as f:
    f.write("# Frontend RemoteApp Browser/Tauri Lifecycle E2E\n\n")
    f.write(f"- Status: `{report['status']}`\n")
    f.write(f"- Surface: `{report['surface']}`\n")
    f.write(f"- Frontend URL: `{report['frontend_url']}`\n")
    f.write(f"- Session id: `{report['session_id']}`\n")
    f.write(f"- Selected Resource URA: `{report['selected_resource_ura']}`\n")
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

if [[ "$SELF_TEST" -eq 1 ]]; then
  python3 - "$EVIDENCE_JSON" <<'PY'
import json
import pathlib
import sys

subject = "easynet:///r/localhost/resource/device.mac-1/streams/window.browser-lifecycle"
session_id = "rd-browser-lifecycle-self-test"
def observed(step, offset):
    step["evidence_source"] = "browser_automation"
    step["component_snapshot_only"] = False
    step["observed_at_ms"] = 1787332000000 + offset
    return step

steps = [
    observed({"name": "app_loaded", "status": "passed"}, 10),
    observed({"name": "authenticated_session", "status": "passed"}, 20),
    observed({"name": "target_picker_opened", "status": "passed"}, 30),
    observed({"name": "permission_status_checked", "status": "passed", "ability": "remote_desktop.permission_status", "subject_ura": None}, 40),
    observed({"name": "consent_granted", "status": "passed", "ability": "remote_desktop.grant_consent", "subject_ura": subject}, 50),
    observed({"name": "session_created", "status": "passed", "ability": "remote_desktop.create_session", "subject_ura": subject, "session_id": session_id}, 60),
    {
        "name": "webrtc_transport_connected",
        "status": "passed",
        "ability": "remote_desktop.set_description",
        "subject_ura": subject,
        "session_id": session_id,
        "rtc_connection_state": "connected",
        "ice_connection_state": "connected",
        "media_stream_attached": True,
        "audio_track_attached": False,
    },
    observed({"name": "watch_events_streaming", "status": "passed", "ability": "remote_desktop.watch_events", "subject_ura": subject, "session_id": session_id}, 80),
    {
        "name": "media_presented",
        "status": "passed",
        "frame_presented": True,
        "media_element_visible": True,
        "frames_presented": 3,
    },
    {
        "name": "media_pipeline_support_visible",
        "status": "passed",
        "visible_label": "pipeline video_only · h264 · bounded_queue_drop_stale_frames · host_audio_not_implemented",
        "media_scope": "video_only",
        "product_ready": False,
        "product_blockers": [
            "host_audio_not_implemented",
            "remoteapp_media_adaptation_e2e_artifact_missing",
        ],
        "selected_route_class": "direct",
        "selected_pair_state": "succeeded",
        "selected_pair_nominated": True,
        "selected_local_candidate_type": "host",
        "selected_remote_candidate_type": "host",
        "selected_candidate_protocol": "udp",
    },
    {
        "name": "input_control_attempted_or_policy_blocked",
        "status": "passed",
        "result": "policy_blocked",
        "blocked_reason": "view_only",
        "visible_status": "input scope view_only · no controls",
        "target_tracking": {
            "status": "resolved",
            "visibility": "visible",
            "focused": None,
            "input_enabled": True,
            "input_blocked_reason": "",
            "focus_epoch": 1,
            "geometry_revision": 1,
        },
    },
    observed({"name": "transport_disconnected", "status": "passed", "subject_ura": subject, "session_id": session_id, "transport_epoch": 2, "old_peer_retired": True, "peer_states": [{"connection": "closed", "ice": "closed"}]}, 120),
    observed({"name": "session_preserved_for_reconnect", "status": "passed", "subject_ura": subject, "session_id": session_id, "same_session": True, "terminal": False, "transport_status": "session preserved for reconnect"}, 130),
    observed({"name": "transport_reconnected", "status": "passed", "ability": "remote_desktop.set_description", "subject_ura": subject, "session_id": session_id, "same_session": True, "prior_transport_epoch": 2, "transport_epoch": 3, "transport_epoch_increased": True, "new_peer_connection": True, "rtc_connection_state": "connected", "ice_connection_state": "connected"}, 140),
    observed({"name": "watch_events_reestablished", "status": "passed", "ability": "remote_desktop.watch_events", "subject_ura": subject, "session_id": session_id, "prior_subscription_count": 1, "subscription_count": 2}, 150),
    observed({"name": "media_presented_after_resume", "status": "passed", "session_id": session_id, "transport_epoch": 3, "frame_presented": True, "media_element_visible": True, "frames_presented": 4, "frame_width": 1280, "frame_height": 720}, 160),
    observed({"name": "input_control_after_resume", "status": "passed", "result": "policy_blocked", "blocked_reason": "view_only", "visible_status": "input scope view_only · no controls"}, 170),
    observed({"name": "session_ended", "status": "passed", "ability": "remote_desktop.end_session", "subject_ura": subject, "session_id": session_id}, 180),
    observed({"name": "terminal_receipt_visible", "status": "passed", "terminal": True, "reason_code": "user_cancelled", "session_id": session_id}, 190),
]
steps[6] = observed(steps[6], 70)
steps[8] = observed(steps[8], 90)
steps[9] = observed(steps[9], 100)
steps[10] = observed(steps[10], 110)
evidence = {
    "status": "passed",
    "proof_mode": "real_browser_tauri_lifecycle",
    "runner_kind": "browser",
    "component_mock": False,
    "real_backend_runtime": True,
    "product_complete_claim": False,
    "frontend_url": "http://127.0.0.1:3000/devices/mac-1",
    "device_ura": "easynet:///r/localhost/device/mac-1",
    "selected_resource_ura": subject,
    "session_id": session_id,
    "transport_resume": {
        "proof_mode": "real_browser_transport_resume",
        "session_id": session_id,
        "subject_ura": subject,
        "same_public_session": True,
        "old_peer_retired": True,
        "new_peer_connection": True,
        "prior_transport_epoch": 2,
        "transport_epoch": 3,
        "transport_epoch_increased": True,
        "watch_events_reestablished": True,
        "frames_presented_after_resume": 4,
        "input_result_before": "policy_blocked",
        "input_result_after": "policy_blocked",
        "input_authority_preserved": True,
    },
    "transport_snapshots": [
        {"session_id": session_id, "subject_ura": subject, "transport_epoch": 2},
        {"session_id": session_id, "subject_ura": subject, "transport_epoch": 3},
    ],
    "steps": steps,
}
pathlib.Path(sys.argv[1]).write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
  validate_evidence
  cp "$EVIDENCE_JSON" "$OUT_DIR/valid-resume-evidence.json"
  python3 - "$EVIDENCE_JSON" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
evidence = json.loads(path.read_text(encoding="utf-8"))
evidence["transport_resume"]["new_peer_connection"] = False
path.write_text(json.dumps(evidence) + "\n", encoding="utf-8")
PY
  if validate_evidence >/dev/null 2>&1; then
    echo "frontend browser lifecycle self-test accepted resume without a new PeerConnection" >&2
    exit 1
  fi
  cp "$OUT_DIR/valid-resume-evidence.json" "$EVIDENCE_JSON"
  validate_evidence
  echo "frontend-remoteapp-browser-lifecycle-e2e self-test ok"
  exit 0
fi

if [[ "$MODE" != "run" ]]; then
  write_report "skipped" "set EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_E2E=1 or pass --run"
  echo "[frontend-remoteapp-browser-lifecycle-e2e] skipped: $REPORT_MD"
  exit 0
fi

if [[ -n "$EVIDENCE_INPUT" ]]; then
  [[ -f "$EVIDENCE_INPUT" ]] || {
    write_report "failed" "evidence json does not exist: $EVIDENCE_INPUT"
    echo "[frontend-remoteapp-browser-lifecycle-e2e] missing evidence json: $EVIDENCE_INPUT" >&2
    exit 1
  }
  cp "$EVIDENCE_INPUT" "$EVIDENCE_JSON"
elif [[ -n "$RUNNER_CMD" ]]; then
  [[ -n "$FRONTEND_URL" ]] || {
    write_report "failed" "--frontend-url is required when --runner-cmd is used"
    echo "[frontend-remoteapp-browser-lifecycle-e2e] --frontend-url is required" >&2
    exit 1
  }
  export EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_EVIDENCE_JSON="$EVIDENCE_JSON"
  export EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_FRONTEND_URL="$FRONTEND_URL"
  export EASYNET_REMOTEAPP_BROWSER_LIFECYCLE_SURFACE="$SURFACE"
  if ! bash -lc "$RUNNER_CMD" >"$RUNNER_STDOUT" 2>"$RUNNER_STDERR"; then
    write_report "failed" "runner command failed"
    echo "[frontend-remoteapp-browser-lifecycle-e2e] runner command failed" >&2
    cat "$RUNNER_STDERR" >&2 || true
    exit 1
  fi
  [[ -f "$EVIDENCE_JSON" ]] || {
    write_report "failed" "runner did not write evidence json"
    echo "[frontend-remoteapp-browser-lifecycle-e2e] runner did not write $EVIDENCE_JSON" >&2
    exit 1
  }
else
  write_report "failed" "--run requires --evidence-json or --runner-cmd"
  echo "[frontend-remoteapp-browser-lifecycle-e2e] --run requires --evidence-json or --runner-cmd" >&2
  exit 1
fi

validate_evidence
echo "[frontend-remoteapp-browser-lifecycle-e2e] PASS: $REPORT_MD"
