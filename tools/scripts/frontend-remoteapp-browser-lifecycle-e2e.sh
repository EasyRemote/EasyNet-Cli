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
PROVENANCE_HELPER="$SELF_DIR/remoteapp-evidence-provenance.py"

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
  permission_status_checked -> [permission_requested] -> consent_granted -> session_created ->
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
  python3 "$PROVENANCE_HELPER" verify --mode "$MODE" --evidence "$EVIDENCE_JSON"
  python3 - "$EVIDENCE_JSON" "$REPORT_JSON" "$REPORT_MD" "$MODE" <<'PY'
import json
import pathlib
import re
import sys

evidence_path, report_path, md_path, mode = sys.argv[1:5]
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

def float_field(obj, key, default=-1.0):
    if not isinstance(obj, dict):
        return default
    try:
        return float(obj.get(key, default))
    except (TypeError, ValueError):
        return default

def fit_even_presentation(native_width, native_height, max_width, max_height):
    if min(native_width, native_height, max_width, max_height) <= 0:
        return (0, 0)
    width = native_width
    height = native_height
    if native_width > max_width or native_height > max_height:
        width_limited_height = (
            native_height * max_width + native_width // 2
        ) // native_width
        height_limited_width = (
            native_width * max_height + native_height // 2
        ) // native_height
        if width_limited_height <= max_height:
            width = max_width
            height = max(1, width_limited_height)
        else:
            width = max(1, height_limited_width)
            height = max_height
    return (width & ~1, height & ~1)

input_event_id_pattern = re.compile(r"^rdinp1_[0-9a-f]{32}$")

def validate_host_input_effects(input_evidence, prefix):
    if evidence.get("host_input_effects_required") is not True:
        return
    effects = input_evidence.get("host_input_effects")
    interactions = input_evidence.get("interaction_sequence")
    require(isinstance(effects, dict),
            f"{prefix}.host_input_effects must be present when host effects are required")
    if not isinstance(effects, dict):
        return
    health = effects.get("observer_health")
    independence = effects.get("observer_independence")
    baseline = effects.get("observer_baseline")
    final = effects.get("observer_final")
    selected_events = effects.get("selected_events")
    correlations = effects.get("event_correlations")
    target_kind = evidence.get("selected_target_kind")
    target_window_ids = (
        [int_field(target_metadata, "window_id", -1)]
        if target_kind == "window"
        else [int(value) for value in target_metadata.get("resolved_window_ids", [])
              if isinstance(value, int) and not isinstance(value, bool) and value > 0]
    )
    target_pid = int_field(
        target_metadata,
        "pid" if target_kind == "window" else "primary_pid",
        -1,
    )
    require(effects.get("observer_schema") == "easynet.remoteapp.linux-x11-sentinel.v1",
            f"{prefix} host observer schema must be canonical")
    require(effects.get("target_kind") == target_kind
            and effects.get("exact_target_effect_observed") is True
            and (effects.get("exact_window_effect_observed") is True
                 if target_kind == "window"
                 else effects.get("exact_application_effect_observed") is True),
            f"{prefix} must prove exact selected-{target_kind} host effects")
    require(int_field(effects, "unexpected_input_event_count", -1) == 0,
            f"{prefix} must prove zero unexpected non-motion input effects")
    require(isinstance(health, dict)
            and health.get("status") == "healthy"
            and int_field(health, "callback_error_count", -1) == 0,
            f"{prefix} host observer must remain healthy")
    require(isinstance(independence, dict)
            and independence.get("proof_mode") == "selected_target_process_x11_callback_log"
            and independence.get("target_pid_matches_observer") is True
            and independence.get("stable_process_instance") is True
            and independence.get("daemon_event_ids_absent_from_observer_log") is True,
            f"{prefix} host observer independence must be derived from target-process evidence")
    require(isinstance(baseline, dict) and isinstance(final, dict),
            f"{prefix} must retain raw baseline/final observer snapshots")
    if isinstance(baseline, dict) and isinstance(final, dict):
        baseline_identity = baseline.get("observer_identity")
        final_identity = final.get("observer_identity")
        identity_fields = (
            "instance_id", "pid", "process_start_ticks", "boot_id", "display",
            "fixture_sha256", "started_at_ms", "event_source",
        )
        require(isinstance(baseline_identity, dict)
                and isinstance(final_identity, dict)
                and all(baseline_identity.get(field) == final_identity.get(field)
                        for field in identity_fields),
                f"{prefix} baseline/final must bind one observer process instance")
        if isinstance(baseline_identity, dict) and isinstance(final_identity, dict):
            require(isinstance(baseline_identity.get("instance_id"), str)
                    and bool(baseline_identity.get("instance_id")),
                    f"{prefix} observer instance_id must be present")
            require(int_field(baseline_identity, "pid", -1) > 0
                    and int_field(baseline_identity, "process_start_ticks", -1) > 0,
                    f"{prefix} observer PID/start ticks must be present")
            require(re.fullmatch(r"[0-9a-f]{64}",
                                 str(baseline_identity.get("fixture_sha256", ""))) is not None,
                    f"{prefix} observer fixture digest must be canonical")
            require(baseline_identity.get("event_source")
                    == "target_process_tk_x11_callbacks",
                    f"{prefix} observer must use target-process X11 callbacks")
            require(int_field(baseline_identity, "pid") == target_pid
                    == int_field(independence, "observer_process_pid")
                    == int_field(independence, "target_process_pid"),
                    f"{prefix} observer PID must own the selected target")
        require(int_field(final, "tick", -1) > int_field(baseline, "tick", -1) > 0,
                f"{prefix} final observer snapshot must advance the same fixture")
        require(int_field(final, "observed_at_ms", -1)
                >= int_field(baseline, "observed_at_ms", -1) > 0,
                f"{prefix} observer snapshot timestamps must be ordered")
        baseline_health = baseline.get("observer_health")
        final_health = final.get("observer_health")
        require(isinstance(baseline_health, dict) and isinstance(final_health, dict)
                and baseline_health.get("status") == "healthy"
                and final_health.get("status") == "healthy"
                and int_field(baseline_health, "callback_error_count", -1) == 0
                and int_field(final_health, "callback_error_count", -1) == 0,
                f"{prefix} raw observer snapshots must both be healthy")
        require(int_field(baseline_health, "event_count", -1)
                == int_field(effects, "baseline_event_count", -2)
                and int_field(final_health, "event_count", -1)
                == int_field(effects, "final_event_count", -2),
                f"{prefix} raw observer counts must bind the correlated event interval")
        baseline_windows = baseline.get("windows")
        final_windows = final.get("windows")
        require(isinstance(baseline_windows, list) and isinstance(final_windows, list),
                f"{prefix} raw observer snapshots must retain windows")
        if isinstance(baseline_windows, list) and isinstance(final_windows, list):
            baseline_selected = [window for window in baseline_windows
                                 if int_field(window, "native_window_id", -1)
                                 in target_window_ids and window.get("viewable") is True]
            final_selected = [window for window in final_windows
                              if int_field(window, "native_window_id", -1)
                              in target_window_ids and window.get("viewable") is True]
            require(len(baseline_selected) == len(target_window_ids)
                    and len(final_selected) == len(target_window_ids)
                    and len(target_window_ids) > 0,
                    f"{prefix} selected {target_kind} surfaces must remain viewable in raw snapshots")
            if target_kind == "window":
                baseline_unrelated = next((window for window in baseline_windows
                                           if window.get("viewable") is True
                                           and int_field(window, "native_window_id", -1)
                                           not in target_window_ids), None)
                final_unrelated = next((window for window in final_windows
                                        if baseline_unrelated is not None
                                        and window.get("viewable") is True
                                        and int_field(window, "native_window_id", -1)
                                        == int_field(baseline_unrelated,
                                                     "native_window_id", -2)), None)
                require(isinstance(baseline_unrelated, dict)
                        and isinstance(final_unrelated, dict),
                        f"{prefix} must retain one stable viewable unrelated Window")
            else:
                unrelated_baselines = effects.get("unrelated_observer_baselines")
                unrelated_finals = effects.get("unrelated_observer_finals")
                require(isinstance(unrelated_baselines, list)
                        and isinstance(unrelated_finals, list)
                        and len(unrelated_baselines) == len(unrelated_finals) > 0,
                        f"{prefix} Application proof must retain independent process observers")
                if isinstance(unrelated_baselines, list) and isinstance(unrelated_finals, list):
                    for observer_baseline in unrelated_baselines:
                        observer_identity = observer_baseline.get("observer_identity", {})
                        observer_pid = int_field(observer_identity, "pid", -1)
                        observer_final = next((candidate for candidate in unrelated_finals
                                               if int_field(candidate.get("observer_identity", {}),
                                                            "pid", -2) == observer_pid), None)
                        require(observer_pid > 0 and observer_pid != target_pid
                                and isinstance(observer_final, dict)
                                and observer_final.get("observer_identity") == observer_identity
                                and int_field(observer_final, "tick", -1)
                                > int_field(observer_baseline, "tick", -1),
                                f"{prefix} unrelated Application observer must be a stable different PID")
    require(int_field(effects, "final_event_count")
            >= int_field(effects, "baseline_event_count") + 4,
            f"{prefix} host observer event count must advance")
    require(sorted(effects.get("selected_native_window_ids", [])) == sorted(target_window_ids),
            f"{prefix} host effects must bind the selected native window set")
    if target_kind == "window":
        require(int_field(effects, "selected_native_window_id") == target_window_ids[0],
                f"{prefix} host effects must bind the selected native window id")
    require(isinstance(selected_events, list) and len(selected_events) == 4,
            f"{prefix} must bind four selected-surface host events")
    require(isinstance(correlations, list) and len(correlations) == 4,
            f"{prefix} must retain four independently derived event correlations")
    require(isinstance(interactions, list) and len(interactions) == 4,
            f"{prefix} interactions must exist for host-effect correlation")
    if not (isinstance(selected_events, list) and len(selected_events) == 4
            and isinstance(correlations, list) and len(correlations) == 4
            and isinstance(interactions, list) and len(interactions) == 4):
        return
    expected = [("pointer", "down"), ("pointer", "up"),
                ("keyboard", "down"), ("keyboard", "up")]
    observed_sequences = []
    runtime_sequences = []
    transport_epochs = []
    for index, (event, correlation, interaction, (kind, action)) in enumerate(
        zip(selected_events, correlations, interactions, expected)
    ):
        event_prefix = f"{prefix}.host_input_effects.selected_events[{index}]"
        frame = interaction.get("submitted_frame") if isinstance(interaction, dict) else None
        applied = interaction.get("applied_event") if isinstance(interaction, dict) else None
        require(isinstance(event, dict), f"{event_prefix} must be an object")
        if not isinstance(event, dict):
            continue
        observed_sequences.append(int_field(event, "sequence"))
        require(event.get("kind") == kind and event.get("action") == action,
                f"{event_prefix} must match {kind}/{action}")
        if target_kind == "window":
            require(event.get("surface") == effects.get("selected_surface"),
                    f"{event_prefix} must remain on the selected surface")
        require(int_field(event, "native_window_id") in target_window_ids,
                f"{event_prefix} must bind the selected native window set")
        require("input_event_id" not in event,
                f"{event_prefix} raw observer event must not contain daemon input_event_id")
        require(isinstance(applied, dict)
                and isinstance(applied.get("input_event_id"), str)
                and input_event_id_pattern.fullmatch(applied["input_event_id"]),
                f"{event_prefix} must bind a canonical daemon input_event_id")
        if isinstance(applied, dict):
            runtime_sequences.append(int_field(applied, "sequence", -1))
            transport_epochs.append(int_field(applied, "transport_epoch", -1))
            require(int_field(applied, "host_received_at_ms") > 0,
                    f"{event_prefix} must retain the daemon host receipt timestamp")
            require(int_field(applied, "host_applied_at_ms") > 0,
                    f"{event_prefix} must retain the daemon host application timestamp")
            require(int_field(applied, "host_applied_at_ms")
                    >= int_field(applied, "host_received_at_ms"),
                    f"{event_prefix} host application must not precede host receipt")
            require(isinstance(correlation, dict)
                    and int_field(correlation, "observer_event_sequence", -1)
                    == int_field(event, "sequence", -2)
                    and int_field(correlation, "daemon_runtime_event_sequence", -1)
                    == int_field(applied, "sequence", -2)
                    and correlation.get("daemon_input_event_id")
                    == applied.get("input_event_id")
                    and int_field(correlation, "host_effect_offset_from_apply_ms", -1000)
                    == int_field(event, "at_ms") - int_field(applied, "host_applied_at_ms"),
                    f"{event_prefix} derived correlation must bind raw and daemon records")
            guard = applied.get("target_guard_validation")
            require(applied.get("safety_release") is False,
                    f"{event_prefix} happy-path input must use normal guarded admission")
            require("safety_release_reason" not in applied,
                    f"{event_prefix} normal input must not expose an emergency-release reason")
            require(isinstance(guard, dict)
                    and guard.get("status") == "passed"
                    and guard.get("session_id") == session_id
                    and guard.get("subject_ura") == subject_ura
                    and guard.get("target_kind") == target_kind
                    and (guard.get("window_id_exact") is True
                         if target_kind == "window"
                         else guard.get("window_set_exact") is True),
                    f"{event_prefix} must retain exact fresh {target_kind} target-guard proof")
            if isinstance(guard, dict):
                require(int_field(guard, "validated_at_ms")
                        >= int_field(guard, "snapshot_started_at_ms") > 0,
                        f"{event_prefix} target guard timestamps must be ordered")
                require(int_field(guard, "target_focus_epoch")
                        == int_field(applied, "target_focus_epoch"),
                        f"{event_prefix} target guard focus epoch must match applied event")
                require(int_field(guard, "target_geometry_revision")
                        == int_field(applied, "target_geometry_revision"),
                        f"{event_prefix} target guard geometry revision must match applied event")
                if target_metadata.get("platform") == "linux":
                    require(isinstance(guard.get("expected_pid"), int)
                            and guard.get("expected_pid", 0) > 0
                            and isinstance(guard.get("expected_process_instance_id"), str)
                            and guard["expected_process_instance_id"].startswith("linux:"),
                            f"{event_prefix} Linux input must prove a boot-scoped process-instance binding")
                    require(guard.get("atomicity") == "x11_server_grab"
                            and int_field(guard, "snapshot_started_at_ms")
                            <= int_field(guard, "guard_acquired_at_ms")
                            <= int_field(guard, "validated_at_ms")
                            <= int_field(guard, "injected_at_ms")
                            <= int_field(guard, "guard_released_at_ms")
                            <= int_field(applied, "host_applied_at_ms"),
                            f"{event_prefix} Linux input must prove ordered atomic X11 validation and injection")
                if kind == "pointer":
                    require(int_field(guard, "pointer_target_window_id")
                            == int_field(event, "native_window_id")
                            and int_field(guard, "pointer_target_window_id") in target_window_ids,
                            f"{event_prefix} pointer guard must bind one selected native window")
                    require(guard.get("pointer_occlusion_checked") is True,
                            f"{event_prefix} pointer guard must prove occlusion checking")
            if kind == "pointer":
                require(applied.get("pointer_position_applied") is True,
                        f"{event_prefix} normal pointer input must apply its submitted position")
        if isinstance(frame, dict):
            require(int_field(event, "at_ms")
                    >= int_field(applied, "host_received_at_ms")
                    >= int_field(frame, "sent_at_ms") > 0,
                    f"{event_prefix} must follow submission and daemon receipt, then occur by 250ms after host application")
            require(int_field(event, "at_ms")
                    <= int_field(applied, "host_applied_at_ms") + 250,
                    f"{event_prefix} must fall inside the 250ms host-effect correlation window")
            if kind == "pointer":
                effect_window = next((window for window in final.get("windows", [])
                                      if int_field(window, "native_window_id", -1)
                                      == int_field(event, "native_window_id", -2)), {})
                expected_x = float_field(frame, "x")
                expected_y = float_field(frame, "y")
                if target_kind == "application":
                    expected_x += float_field(target_metadata, "union_x") \
                        - float_field(effect_window, "x")
                    expected_y += float_field(target_metadata, "union_y") \
                        - float_field(effect_window, "y")
                require(abs(float_field(event, "x") - expected_x) <= 8
                        and abs(float_field(event, "y") - expected_y) <= 8,
                        f"{event_prefix} pointer coordinates must match the submitted target point")
            else:
                require(str(event.get("keysym", "")).lower()
                        == str(frame.get("key", "")).lower(),
                        f"{event_prefix} key symbol must match the submitted key")
    require(all(sequence > 0 for sequence in observed_sequences)
            and observed_sequences == sorted(set(observed_sequences)),
            f"{prefix} selected host-event sequences must strictly advance")
    require(all(sequence > 0 for sequence in runtime_sequences)
            and runtime_sequences == sorted(set(runtime_sequences)),
            f"{prefix} daemon applied-event sequences must strictly advance")
    require(all(epoch > 0 for epoch in transport_epochs)
            and len(set(transport_epochs)) == 1,
            f"{prefix} correlated input must remain on one positive transport epoch")

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
expected_origin = "contract_self_test" if mode == "self-test" else "live_runner"
ability_steps = {
    "permission_status_checked": "remote_desktop.permission_status",
    "permission_requested": "remote_desktop.request_permission",
    "consent_granted": "remote_desktop.grant_consent",
    "session_created": "remote_desktop.create_session",
    "webrtc_transport_connected": "remote_desktop.set_description",
    "watch_events_streaming": "remote_desktop.watch_events",
    "session_ended": "remote_desktop.end_session",
}

require(evidence.get("status") == "passed", "evidence.status must be passed")
require(evidence.get("evidence_origin") == expected_origin,
        f"evidence_origin must be {expected_origin}")
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
target_kind = evidence.get("selected_target_kind")
target_snapshot = evidence.get("selected_target_snapshot")
target_metadata = target_snapshot.get("metadata") if isinstance(target_snapshot, dict) else {}
if not isinstance(target_metadata, dict):
    target_metadata = {}
target_execution = evidence.get("target_execution_snapshot")
session_id = evidence.get("session_id")
require(isinstance(device_ura, str) and device_ura.startswith("easynet:///"),
        "device_ura must be a canonical EasyNet URA")
require(isinstance(subject_ura, str) and subject_ura.startswith("easynet:///"),
        "selected_resource_ura must be a canonical EasyNet Resource URA")
require(isinstance(session_id, str) and session_id,
        "session_id must be recorded")

if target_kind == "application":
    require(isinstance(target_snapshot, dict),
            "application target must include selected_target_snapshot")
    if not isinstance(target_snapshot, dict):
        target_snapshot = {}
    metadata = target_snapshot.get("metadata")
    require(target_snapshot.get("resource_ura") == subject_ura,
            "application target snapshot must bind selected_resource_ura")
    require(target_snapshot.get("type") == "application",
            "application target snapshot type must be application")
    require(isinstance(metadata, dict),
            "application target snapshot metadata must be present")
    if not isinstance(metadata, dict):
        metadata = {}
    resolved_window_ids = metadata.get("resolved_window_ids")
    require(metadata.get("capture_target") == "application",
            "application target capture_target must remain application")
    require(metadata.get("discovery_scope") in {
                "application_window_set",
                "process_window_set",
            },
            "application target discovery_scope must be an exact application window set")
    require(metadata.get("display_scoped", False) is False,
            "application target must not be display scoped")
    require(metadata.get("display_id") is None,
            "application target must not carry one display capture id")
    require(isinstance(resolved_window_ids, list) and bool(resolved_window_ids)
            and all(isinstance(value, int) and not isinstance(value, bool) and value > 0
                    for value in resolved_window_ids),
            "application target must bind non-empty positive resolved_window_ids")
    if isinstance(resolved_window_ids, list):
        require(len(set(resolved_window_ids)) == len(resolved_window_ids),
                "application target resolved_window_ids must be unique")
        require(metadata.get("window_count") == len(resolved_window_ids),
                "application target window_count must match resolved_window_ids")
    require(int_field(metadata, "window_set_epoch") > 0,
            "application target window_set_epoch must be positive")
    require(int_field(metadata, "surface_layout_epoch") > 0,
            "application target surface_layout_epoch must be positive")
    require(isinstance(target_execution, dict),
            "application target must include target_execution_snapshot")
    if not isinstance(target_execution, dict):
        target_execution = {}
    execution_binding = target_execution.get("target_binding")
    execution_scope = target_execution.get("scope_audit")
    require(target_execution.get("session_id") == session_id,
            "application execution snapshot must bind session_id")
    require(target_execution.get("subject_ura") == subject_ura,
            "application execution snapshot must bind selected_resource_ura")
    require(isinstance(execution_binding, dict),
            "application execution target_binding must be present")
    require(isinstance(execution_scope, dict),
            "application execution scope_audit must be present")
    if not isinstance(execution_binding, dict):
        execution_binding = {}
    if not isinstance(execution_scope, dict):
        execution_scope = {}
    native_locator = execution_binding.get("native_locator")
    capture_proof = execution_binding.get("capture_proof")
    app_window_set = execution_binding.get("app_window_set")
    require(execution_binding.get("subject_ura") == subject_ura,
            "application execution target_binding must bind selected_resource_ura")
    require(execution_binding.get("target_kind") == "application",
            "application execution target_kind must remain application")
    require(execution_binding.get("capture_scope") == "AppSurface",
            "application execution capture_scope must be AppSurface")
    require(isinstance(native_locator, dict) and native_locator.get("display_id") is None,
            "application execution native locator must not become display scoped")
    require(isinstance(capture_proof, dict)
            and capture_proof.get("target_kind") == "application"
            and capture_proof.get("display_id") is None,
            "application execution capture proof must remain application scoped")
    require(isinstance(app_window_set, dict)
            and isinstance(app_window_set.get("resolved_window_ids"), list)
            and bool(app_window_set.get("resolved_window_ids")),
            "application execution must bind one non-empty app window set")
    require(execution_scope.get("requested_target_kind") == "application"
            and execution_scope.get("effective_target_kind") == "application",
            "application execution scope audit must preserve requested/effective kind")
    require(execution_scope.get("capture_surface") == "AppSurface",
            "application execution scope audit must use AppSurface")
    require(execution_scope.get("scope_widened") is False,
            "application execution scope must not widen")
    require(execution_scope.get("display_fallback_used") is False,
            "application execution must not use display fallback")

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

permission_requested = step_by_name.get("permission_requested")
if isinstance(permission_requested, dict):
    permission_status_index = step_names.index("permission_status_checked")
    permission_request_index = step_names.index("permission_requested")
    consent_index = step_names.index("consent_granted")
    require(permission_status_index < permission_request_index < consent_index,
            "permission_requested must follow permission_status_checked and precede consent_granted")
    require(permission_requested.get("capture_state") == "granted",
            "permission_requested.capture_state must be granted in passing lifecycle evidence")

for step_name, ability in ability_steps.items():
    step = step_by_name.get(step_name)
    if not isinstance(step, dict):
        continue
    require(step.get("ability") == ability, f"{step_name}: ability must be {ability}")
    if step_name == "permission_status_checked":
        require(step.get("subject_ura") in {None, ""},
                "permission_status_checked must be host-local and not target-scoped")
    elif step_name == "permission_requested":
        require(step.get("subject_ura") in {None, ""},
                "permission_requested must be host-local and not target-scoped")
    else:
        require(step.get("subject_ura") == subject_ura,
                f"{step_name}: subject_ura must equal selected Resource URA")

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
    audio_blockers = [
        blocker for blocker in blockers
        if blocker != "remoteapp_media_adaptation_e2e_artifact_missing"
    ] if isinstance(blockers, list) else []
    require(bool(audio_blockers),
            "video_only media must expose the effective target/platform audio blocker")
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
        "target_focus_unobserved",
        "target_hidden",
        "target_minimized",
        "target_lost",
        "target_stale",
        "target_unresolved",
        "target_rebinding",
        "target_invalidated",
        "target_scoped_keyboard_pointer_dispatch_unsafe",
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
        target_lifecycle_blockers = {
            "target_input_not_ready",
            "target_focus_unobserved",
            "target_hidden",
            "target_minimized",
            "target_lost",
            "target_stale",
            "target_unresolved",
            "target_rebinding",
            "target_invalidated",
        }
        if blocked_reason in target_lifecycle_blockers:
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
    interaction_sequence = input_step.get("interaction_sequence")
    require(isinstance(interaction_sequence, list) and len(interaction_sequence) == 4,
            "input_applied must prove pointer down/up and key down/up interaction_sequence")
    if isinstance(interaction_sequence, list) and len(interaction_sequence) == 4:
        expected_interactions = [
            ("pointer", "down"),
            ("pointer", "up"),
            ("key", "down"),
            ("key", "up"),
        ]
        previous_sequence = 0
        for index, ((expected_type, expected_action), interaction) in enumerate(
            zip(expected_interactions, interaction_sequence)
        ):
            prefix = f"interaction_sequence[{index}]"
            require(isinstance(interaction, dict), f"{prefix} must be an object")
            if not isinstance(interaction, dict):
                continue
            interaction_sequence_number = int_field(interaction, "client_sequence")
            interaction_frame = interaction.get("submitted_frame")
            interaction_event = interaction.get("applied_event")
            require(interaction_sequence_number > previous_sequence,
                    f"{prefix} client_sequence must strictly advance")
            previous_sequence = interaction_sequence_number
            require(0 <= float_field(interaction, "latency_ms") <= 250,
                    f"{prefix} latency_ms must be within frontend lifecycle bound")
            require(isinstance(interaction_frame, dict), f"{prefix} submitted_frame must be an object")
            if isinstance(interaction_frame, dict):
                require(interaction_frame.get("type") == expected_type,
                        f"{prefix} submitted_frame.type must be {expected_type}")
                require(interaction_frame.get("action") == expected_action,
                        f"{prefix} submitted_frame.action must be {expected_action}")
                require(int_field(interaction_frame, "client_sequence") == interaction_sequence_number,
                        f"{prefix} submitted_frame sequence must match")
            require(isinstance(interaction_event, dict), f"{prefix} applied_event must be an object")
            if isinstance(interaction_event, dict):
                require(interaction_event.get("event_type") in {"INPUT_FRAME_APPLIED", "input_frame_applied"},
                        f"{prefix} must prove INPUT_FRAME_APPLIED")
                require(interaction_event.get("session_id") == session_id,
                        f"{prefix} applied_event must bind the created session")
                require(int_field(interaction_event, "client_sequence") == interaction_sequence_number,
                        f"{prefix} applied_event sequence must match")
    validate_host_input_effects(input_step, "input_control_attempted_or_policy_blocked")
    focus_recovery = input_step.get("focus_recovery")
    if focus_recovery is not None:
        require(isinstance(focus_recovery, dict),
                "input_applied focus_recovery must be an object")
        if isinstance(focus_recovery, dict):
            require(focus_recovery.get("ability") == "remote_desktop.focus_target",
                    "focus_recovery must bind remote_desktop.focus_target")
            require(focus_recovery.get("invocation_observed") is True,
                    "focus_recovery must prove the focus Ability invocation")
            require(int_field(focus_recovery, "invocation_count") > 0,
                    "focus_recovery invocation_count must be positive")
            prior_focus_epoch = int_field(focus_recovery, "prior_target_focus_epoch")
            committed_focus_epoch = int_field(focus_recovery, "committed_target_focus_epoch")
            require(prior_focus_epoch > 0,
                    "focus_recovery prior_target_focus_epoch must be positive")
            require(committed_focus_epoch == target_focus_epoch,
                    "focus_recovery committed_target_focus_epoch must match input_applied target_focus_epoch")
            require(committed_focus_epoch > prior_focus_epoch,
                    "focus_recovery must prove the committed target focus epoch advanced")

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

terminal_crash_replay = evidence.get("terminal_crash_replay")
if terminal_crash_replay is not None:
    require(isinstance(terminal_crash_replay, dict),
            "terminal_crash_replay must be an object when present")
    require(transport_resume is None,
            "terminal crash replay and transport resume must be separate scenarios")
    if isinstance(terminal_crash_replay, dict):
        crash_steps = ["terminal_crash_armed", "terminal_crash_observed", "session_ended"]
        crash_cursor = step_names.index("input_control_attempted_or_policy_blocked")
        for crash_step in crash_steps:
            try:
                crash_index = step_names.index(crash_step)
            except ValueError:
                errors.append(f"missing terminal crash replay step: {crash_step}")
                continue
            require(crash_index > crash_cursor,
                    f"terminal crash replay step order is wrong at {crash_step}")
            crash_cursor = crash_index
        require(terminal_crash_replay.get("proof_mode")
                == "real_browser_terminal_promotion_crash_replay",
                "terminal_crash_replay.proof_mode must identify terminal promotion crash replay")
        require(terminal_crash_replay.get("session_id") == session_id,
                "terminal_crash_replay must preserve the created session id")
        require(terminal_crash_replay.get("subject_ura") == subject_ura,
                "terminal_crash_replay must remain bound to the selected Resource URA")
        for field in (
            "same_public_session",
            "end_session_request_observed",
            "response_lost_to_daemon_crash",
            "device_offline_observed",
            "device_online_after_restart",
            "show_session_replayed_terminal",
            "terminal",
        ):
            require(terminal_crash_replay.get(field) is True,
                    f"terminal_crash_replay.{field} must be true")
        require(terminal_crash_replay.get("reason_code") == "caller_ended",
                "terminal crash replay must preserve the real product End reason caller_ended")
        require(int_field(terminal_crash_replay, "show_session_count_after")
                > int_field(terminal_crash_replay, "show_session_count_before"),
                "terminal crash replay must prove a new public show_session observation")
        require(int_field(terminal_crash_replay, "end_session_request_observed_at_ms") > 0,
                "terminal crash replay must timestamp the original end_session request")
        require(int_field(terminal_crash_replay, "terminal_replayed_at_ms")
                > int_field(terminal_crash_replay, "end_session_request_observed_at_ms"),
                "terminal replay must occur after the original end_session request")
        armed = step_by_name.get("terminal_crash_armed", {})
        crashed = step_by_name.get("terminal_crash_observed", {})
        require(armed.get("fault") == "crash_after_terminal_promotion",
                "terminal_crash_armed must identify the exact promotion boundary")
        require(armed.get("session_id") == session_id and armed.get("subject_ura") == subject_ura,
                "terminal_crash_armed must bind session and Resource subject")
        require(crashed.get("session_id") == session_id
                and crashed.get("device_online") == "false",
                "terminal_crash_observed must preserve the session while Device is offline")
        require(ended.get("response_lost_to_daemon_crash") is True,
                "session_ended must expose the lost end_session response")
        require(terminal.get("recovered_through") == "remote_desktop.show_session",
                "terminal receipt UI must recover through public show_session")
        snapshots = evidence.get("device_state_snapshots")
        require(isinstance(snapshots, list) and snapshots,
                "terminal crash replay must include Device state snapshots")
        if isinstance(snapshots, list):
            state_codes = [
                snapshot.get("state_code")
                for snapshot in snapshots
                if isinstance(snapshot, dict)
            ]
            try:
                offline_index = state_codes.index("C440")
            except ValueError:
                offline_index = -1
            require(offline_index >= 0,
                    "terminal crash replay must observe Device C440")
            require("J700" in state_codes[offline_index + 1:],
                    "terminal crash replay must observe Device J700 after C440")

target_monitor_worker_recovery = evidence.get("target_monitor_worker_recovery")
if target_monitor_worker_recovery is not None:
    require(isinstance(target_monitor_worker_recovery, dict),
            "target_monitor_worker_recovery must be an object when present")
    require(transport_resume is None and terminal_crash_replay is None,
            "target-monitor worker recovery must be a separate lifecycle scenario")
    if isinstance(target_monitor_worker_recovery, dict):
        worker_steps = [
            "target_monitor_crash_armed",
            "target_monitor_recovery_media_presented",
            "target_monitor_recovered",
        ]
        worker_cursor = step_names.index("input_control_attempted_or_policy_blocked")
        for worker_step in worker_steps:
            try:
                worker_index = step_names.index(worker_step)
            except ValueError:
                errors.append(f"missing target-monitor recovery step: {worker_step}")
                continue
            require(worker_index > worker_cursor,
                    f"target-monitor recovery step order is wrong at {worker_step}")
            worker_cursor = worker_index
        require(worker_cursor < step_names.index("session_ended"),
                "target-monitor recovery must complete before terminal cleanup")
        require(target_monitor_worker_recovery.get("proof_mode")
                == "real_browser_target_monitor_worker_recovery",
                "target-monitor recovery proof mode must be live Browser worker recovery")
        require(target_monitor_worker_recovery.get("session_id") == session_id,
                "target-monitor recovery must preserve the created session id")
        require(target_monitor_worker_recovery.get("subject_ura") == subject_ura,
                "target-monitor recovery must remain bound to the selected Resource URA")
        require(target_monitor_worker_recovery.get("same_public_session") is True,
                "target-monitor recovery must preserve one public session")
        expected_worker_events = [
            "PLUGIN_WORKER_CRASHED",
            "PLUGIN_WORKER_RESTARTED",
            "TARGET_MONITOR_RESTARTED",
        ]
        require(target_monitor_worker_recovery.get("ordered_worker_events")
                == expected_worker_events,
                "target-monitor recovery events must be exact and ordered")
        worker_event_records = target_monitor_worker_recovery.get("worker_event_records")
        require(isinstance(worker_event_records, list) and len(worker_event_records) == 3,
                "target-monitor recovery must expose three public worker event records")
        if isinstance(worker_event_records, list) and len(worker_event_records) == 3:
            require([record.get("event_type") for record in worker_event_records]
                    == expected_worker_events,
                    "public worker event records must be exact and ordered")
            sequences = [record.get("sequence") for record in worker_event_records]
            require(all(isinstance(sequence, int) and sequence > 0 for sequence in sequences)
                    and sequences == sorted(set(sequences)),
                    "public worker event sequences must be strictly ordered")
            payloads = [record.get("payload") for record in worker_event_records]
            require(all(isinstance(payload, dict)
                        and payload.get("component") == "target_monitor"
                        for payload in payloads),
                    "public worker events must bind the target_monitor component")
            if all(isinstance(payload, dict) for payload in payloads):
                failed_generation = payloads[0].get("failed_generation")
                restarted_generation = payloads[1].get("restarted_generation")
                require(isinstance(failed_generation, int) and failed_generation > 0,
                        "public worker crash generation must be positive")
                require(all(payload.get("failed_generation") == failed_generation
                            for payload in payloads),
                        "public worker events must bind one failed generation")
                require(isinstance(restarted_generation, int)
                        and restarted_generation > failed_generation,
                        "public worker replacement generation must increase")
                require(payloads[2].get("restarted_generation") == restarted_generation,
                        "functional target-monitor recovery must bind the replacement generation")
        for field in (
            "daemon_transport_epoch_preserved",
            "target_binding_epoch_preserved",
            "media_source_epoch_preserved",
            "consent_epoch_preserved",
        ):
            require(target_monitor_worker_recovery.get(field) is True,
                    f"target_monitor_worker_recovery.{field} must be true")
        for prefix in ("transport_epoch", "binding_epoch", "media_source_epoch", "consent_epoch"):
            require(int_field(target_monitor_worker_recovery, f"{prefix}_before") > 0,
                    f"target-monitor recovery {prefix}_before must be positive")
            require(int_field(target_monitor_worker_recovery, f"{prefix}_after")
                    == int_field(target_monitor_worker_recovery, f"{prefix}_before"),
                    f"target-monitor recovery must preserve {prefix}")
        require(int_field(target_monitor_worker_recovery, "frames_rendered_after_worker_restart") > 0,
                "target-monitor recovery must render a frame after restart")
        require(int_field(target_monitor_worker_recovery,
                          "first_frame_rendered_after_worker_restart_at_ms") > 0,
                "target-monitor recovery must timestamp its post-restart frame")
        require(target_monitor_worker_recovery.get("new_consent_required") is False,
                "target-monitor recovery must not mint new consent")
        require("target monitor recovered" in str(
                    target_monitor_worker_recovery.get("frontend_status", "")),
                "frontend must visibly project target-monitor recovery")
        armed = step_by_name.get("target_monitor_crash_armed", {})
        recovered_step = step_by_name.get("target_monitor_recovered", {})
        require(armed.get("fault") == "crash_target_monitor_generation",
                "target-monitor arm must identify the worker-generation fault")
        require(armed.get("session_id") == session_id and armed.get("subject_ura") == subject_ura,
                "target-monitor arm must bind session and Resource subject")
        require(recovered_step.get("worker_events") == expected_worker_events,
                "target-monitor recovered step must expose ordered public events")

application_target_churn = evidence.get("application_target_churn")
if application_target_churn is not None:
    require(isinstance(application_target_churn, dict),
            "application_target_churn must be an object when present")
    require(target_kind == "application",
            "application target churn requires selected_target_kind=application")
    require(transport_resume is None and terminal_crash_replay is None
            and target_monitor_worker_recovery is None,
            "application target churn must be a separate lifecycle scenario")
    if isinstance(application_target_churn, dict):
        churn_mode = application_target_churn.get("churn_mode", "window_set")
        require(churn_mode in {"window_set", "geometry"},
                "application target churn mode must be window_set or geometry")
        if churn_mode == "geometry":
            media_step_name = "media_presented_after_application_geometry_rebind"
            input_step_name = "input_applied_after_application_geometry_rebind"
            rebound_step_name = "application_geometry_rebound"
            expected_proof_mode = "real_application_geometry_churn"
        else:
            media_step_name = "media_presented_after_application_window_set_rebind"
            input_step_name = "input_applied_after_application_window_set_rebind"
            rebound_step_name = "application_window_set_rebound"
            expected_proof_mode = "real_application_window_set_churn"
        churn_steps = [
            media_step_name,
            input_step_name,
            rebound_step_name,
        ]
        churn_cursor = step_names.index("input_control_attempted_or_policy_blocked")
        for churn_step in churn_steps:
            try:
                churn_index = step_names.index(churn_step)
            except ValueError:
                errors.append(f"missing application target churn step: {churn_step}")
                continue
            require(churn_index > churn_cursor,
                    f"application target churn step order is wrong at {churn_step}")
            churn_cursor = churn_index
        require(churn_cursor < step_names.index("session_ended"),
                "application target churn must complete before terminal cleanup")
        require(application_target_churn.get("proof_mode") == expected_proof_mode,
                f"application target churn proof_mode must be {expected_proof_mode}")
        require(application_target_churn.get("session_id") == session_id,
                "application target churn must preserve the created session id")
        require(application_target_churn.get("selected_resource_ura") == subject_ura,
                "application target churn must remain bound to the selected Resource URA")
        binding_before = int_field(application_target_churn, "binding_epoch_before")
        binding_after = int_field(application_target_churn, "binding_epoch_after")
        identity_before = int_field(application_target_churn, "target_identity_epoch_before")
        identity_after = int_field(application_target_churn, "target_identity_epoch_after")
        geometry_before = int_field(application_target_churn, "target_geometry_revision_before")
        geometry_after = int_field(application_target_churn, "target_geometry_revision_after")
        require(binding_before > 0 and binding_after > binding_before,
                "application target churn binding_epoch must advance")
        if churn_mode == "geometry":
            require(identity_before > 0 and identity_after == identity_before,
                    "application geometry churn must preserve the target identity epoch")
        else:
            require(identity_before > 0 and identity_after > 0
                    and identity_after != identity_before,
                    "application window-set churn identity epoch must change")
        require(geometry_before > 0 and geometry_after > geometry_before,
                "application target churn geometry revision must advance")
        original_window_ids = application_target_churn.get("resolved_window_ids_before")
        rebound_window_ids = application_target_churn.get("resolved_window_ids_after")
        for label, window_ids in (
            ("original", original_window_ids), ("rebound", rebound_window_ids)
        ):
            require(isinstance(window_ids, list) and len(window_ids) >= 2
                    and all(isinstance(value, int) and not isinstance(value, bool) and value > 0
                            for value in window_ids),
                    f"application target churn must expose at least two positive {label} native window ids")
            if isinstance(window_ids, list):
                require(len(set(window_ids)) == len(window_ids),
                        f"application target churn {label} native window ids must be unique")
        if isinstance(original_window_ids, list) and isinstance(rebound_window_ids, list):
            if churn_mode == "geometry":
                require(rebound_window_ids == original_window_ids,
                        "application geometry churn must preserve the committed native window set")
            else:
                require(rebound_window_ids != original_window_ids,
                        "application window-set churn must replace a native window identity")
        target_events = application_target_churn.get("target_events", [])
        if churn_mode == "geometry":
            require(target_events == ["TARGET_MOVED", "TARGET_RESIZED"],
                    "application geometry churn must expose ordered TARGET_MOVED and TARGET_RESIZED events")
            lifecycle_events = evidence.get("target_lifecycle_events")
            require(isinstance(lifecycle_events, list),
                    "application geometry churn must include target_lifecycle_events")
            geometry_events = [event for event in lifecycle_events
                if isinstance(event, dict)
                and event.get("session_id") == session_id
                and event.get("subject_ura") == subject_ura]
            require(len(geometry_events) == 2,
                    "application geometry churn must include exactly two bound lifecycle event records")
            if len(geometry_events) == 2:
                require([event.get("event_type") for event in geometry_events] == target_events,
                        "application geometry lifecycle records must match the ordered event summary")
                sequences = [int_field(event, "sequence") for event in geometry_events]
                require(sequences[0] > 0 and sequences[1] == sequences[0] + 1,
                        "application geometry lifecycle event sequences must be consecutive")
                binding_ids = {event.get("binding_id") for event in geometry_events}
                transport_epochs = {int_field(event, "transport_epoch") for event in geometry_events}
                media_source_epochs = {int_field(event, "media_source_epoch") for event in geometry_events}
                require(len(binding_ids) == 1 and None not in binding_ids,
                        "application geometry lifecycle events must bind one committed binding id")
                require(len(transport_epochs) == 1 and min(transport_epochs) > 0,
                        "application geometry lifecycle events must bind one transport epoch")
                require(len(media_source_epochs) == 1 and min(media_source_epochs) > 0,
                        "application geometry lifecycle events must bind one media-source epoch")
                for event in geometry_events:
                    payload = event.get("payload")
                    require(event.get("source_ability") == "remote_desktop.show_session",
                            "application geometry lifecycle event must come from show_session")
                    require(event.get("terminal") is False,
                            "application geometry lifecycle event must be non-terminal")
                    require(int_field(event, "binding_epoch") == binding_after
                            and int_field(event, "target_identity_epoch") == identity_after
                            and int_field(event, "target_geometry_revision") == geometry_after,
                            "application geometry lifecycle event must bind committed epochs")
                    require(isinstance(payload, dict),
                            "application geometry lifecycle event payload must be an object")
                    if isinstance(payload, dict):
                        event_binding = payload.get("target_binding", {})
                        require(payload.get("subject_ura") == subject_ura,
                                "application geometry lifecycle payload must bind the Resource subject")
                        require(int_field(payload, "previous_binding_epoch") == binding_before
                                and int_field(payload, "previous_target_identity_epoch") == identity_before
                                and int_field(payload, "previous_target_geometry_revision") == geometry_before,
                                "application geometry lifecycle payload must bind previous epochs")
                        require(int_field(payload, "binding_epoch") == binding_after
                                and int_field(payload, "target_identity_epoch") == identity_after
                                and int_field(payload, "target_geometry_revision") == geometry_after,
                                "application geometry lifecycle payload must bind committed epochs")
                        require(event_binding.get("binding_id") == event.get("binding_id")
                                and int_field(event_binding, "binding_epoch") == binding_after
                                and int_field(event_binding, "target_geometry_revision") == geometry_after,
                                "application geometry lifecycle payload must bind the committed target binding")
                        require(event_binding.get("app_window_set", {})
                                    .get("resolved_window_ids") == rebound_window_ids,
                                "application geometry lifecycle payload must preserve the native window set")
        require(int_field(application_target_churn, "frames_rendered_after_rebind") > 0,
                "application target churn must render media after rebind")
        require(application_target_churn.get("scope_widened") is False,
                "application target churn must not widen capture scope")
        require(application_target_churn.get("display_fallback_used") is False,
                "application target churn must not use display fallback")
        churn_input = application_target_churn.get("input_after_rebind")
        require(isinstance(churn_input, dict)
                and churn_input.get("result") == "input_applied",
                "application target churn must apply input after rebind")
        if isinstance(churn_input, dict):
            require(int_field(churn_input, "target_geometry_revision") == geometry_after,
                    "application target churn input must bind the rebound geometry revision")
            interactions = churn_input.get("interaction_sequence")
            require(isinstance(interactions, list) and len(interactions) == 4,
                    "application target churn input must prove pointer/key down/up")
            if isinstance(interactions, list) and len(interactions) == 4:
                expected = [("pointer", "down"), ("pointer", "up"),
                            ("key", "down"), ("key", "up")]
                sequences = []
                for index, (interaction, (frame_type, action)) in enumerate(zip(interactions, expected)):
                    prefix = f"application_target_churn.input_after_rebind.interaction_sequence[{index}]"
                    require(isinstance(interaction, dict), f"{prefix} must be an object")
                    if not isinstance(interaction, dict):
                        continue
                    frame = interaction.get("submitted_frame")
                    applied = interaction.get("applied_event")
                    sequence = int_field(interaction, "client_sequence")
                    sequences.append(sequence)
                    require(isinstance(frame, dict)
                            and frame.get("type") == frame_type
                            and frame.get("action") == action
                            and int_field(frame, "client_sequence") == sequence,
                            f"{prefix} must bind the expected submitted frame")
                    require(isinstance(applied, dict)
                            and applied.get("event_type") in {"INPUT_FRAME_APPLIED", "input_frame_applied"}
                            and applied.get("session_id") == session_id
                            and int_field(applied, "client_sequence") == sequence
                            and int_field(applied, "target_geometry_revision") == geometry_after,
                            f"{prefix} must bind the daemon applied event")
                require(all(sequence > 0 for sequence in sequences)
                        and sequences == sorted(set(sequences)),
                        "application target churn input sequences must strictly advance")
                initial_input = step_by_name.get("input_control_attempted_or_policy_blocked", {})
                initial_interactions = initial_input.get("interaction_sequence", [])
                initial_sequences = [int_field(item, "client_sequence")
                    for item in initial_interactions if isinstance(item, dict)]
                require(not initial_sequences or sequences[0] > max(initial_sequences),
                        "application target churn input must follow the initial interaction sequence")
            input_probe = churn_input.get("input_probe")
            require(isinstance(input_probe, dict)
                    and input_probe.get("source") == "committed_application_surface_center"
                    and input_probe.get("window_id") in rebound_window_ids,
                    "application target churn input probe must select a committed native surface")
        execution_snapshots = evidence.get("target_execution_snapshots")
        require(isinstance(execution_snapshots, list) and bool(execution_snapshots),
                "application target churn must include target_execution_snapshots")
        initial_snapshot = None
        rebound_snapshot = None
        if isinstance(execution_snapshots, list):
            initial_snapshot = next((snapshot for snapshot in execution_snapshots
                if isinstance(snapshot, dict)
                and snapshot.get("session_id") == session_id
                and int_field(snapshot.get("target_binding", {}), "binding_epoch") == binding_before
                and snapshot.get("target_binding", {}).get("app_window_set", {})
                    .get("resolved_window_ids") == original_window_ids), None)
            rebound_snapshot = next((snapshot for snapshot in reversed(execution_snapshots)
                if isinstance(snapshot, dict)
                and snapshot.get("session_id") == session_id
                and int_field(snapshot.get("target_binding", {}), "binding_epoch") >= binding_after
                and snapshot.get("target_binding", {}).get("app_window_set", {})
                    .get("resolved_window_ids") == rebound_window_ids), None)
        require(isinstance(initial_snapshot, dict),
                "application target churn must expose the initial committed capture generation")
        require(isinstance(rebound_snapshot, dict),
                "application target churn must expose the rebound window set through show_session")
        if isinstance(rebound_snapshot, dict):
            rebound_scope = rebound_snapshot.get("scope_audit", {})
            rebound_binding = rebound_snapshot.get("target_binding", {})
            rebound_proof = rebound_binding.get("capture_proof", {})
            rebound_layout = rebound_binding.get("app_surface_layout")
            require(rebound_scope.get("scope_widened") is False
                    and rebound_scope.get("display_fallback_used") is False,
                    "application target churn show_session snapshot must preserve application scope")
            require(isinstance(rebound_proof, dict)
                    and int_field(rebound_proof, "native_width") > 0
                    and int_field(rebound_proof, "native_height") > 0,
                    "application target churn must publish positive native capture dimensions")
            require(isinstance(rebound_layout, dict)
                    and rebound_proof.get("app_surface_layout") == rebound_layout,
                    "application target churn capture proof must bind the committed surface layout")
            require(rebound_proof.get("app_window_set", {})
                        .get("resolved_window_ids") == rebound_window_ids,
                    "application target churn capture proof must bind the committed native window set")
            initial_proof = initial_snapshot.get("target_binding", {}).get("capture_proof", {}) \
                if isinstance(initial_snapshot, dict) else {}
            require(int_field(rebound_proof, "verified_at_ms")
                    > int_field(initial_proof, "verified_at_ms"),
                    "application target churn capture proof must be reverified after the initial generation")

            # Linux/X11 xcap geometry and captured pixels share one physical
            # coordinate space. Enforce the exact union there so a negotiated
            # coded resolution can never masquerade as native target evidence.
            # Do not generalize this equality to Retina/Windows DPI surfaces.
            selected_platform = metadata.get("platform") if isinstance(metadata, dict) else None
            if selected_platform == "linux" and rebound_proof.get("backend") == "xcap":
                surfaces = rebound_layout.get("front_to_back_surfaces", []) \
                    if isinstance(rebound_layout, dict) else []
                valid_surfaces = isinstance(surfaces, list) and bool(surfaces) and all(
                    isinstance(surface, dict)
                    and int_field(surface, "window_id") > 0
                    and int_field(surface, "width") > 0
                    and int_field(surface, "height") > 0
                    for surface in surfaces
                )
                require(valid_surfaces,
                        "Linux application churn must publish a valid native surface layout")
                if valid_surfaces:
                    min_x = min(int_field(surface, "x") for surface in surfaces)
                    min_y = min(int_field(surface, "y") for surface in surfaces)
                    max_x = max(int_field(surface, "x") + int_field(surface, "width")
                                for surface in surfaces)
                    max_y = max(int_field(surface, "y") + int_field(surface, "height")
                                for surface in surfaces)
                    require((int_field(rebound_proof, "native_width"),
                             int_field(rebound_proof, "native_height"))
                            == (max_x - min_x, max_y - min_y),
                            "Linux xcap application native proof must match the exact surface-layout union, not the coded presentation resolution")
        rebound_step = step_by_name.get(rebound_step_name, {})
        require(rebound_step.get("session_id") == session_id
                and rebound_step.get("subject_ura") == subject_ura,
                f"{rebound_step_name} step must bind session and Resource subject")
        require(int_field(rebound_step, "binding_epoch_before") == binding_before
                and int_field(rebound_step, "binding_epoch_after") == binding_after,
                f"{rebound_step_name} step must match churn binding epochs")
        require(rebound_step.get("resolved_window_ids") == rebound_window_ids,
                f"{rebound_step_name} step must match rebound native windows")
        require(rebound_step.get("input_applied_after_rebind") is True,
                f"{rebound_step_name} step must prove post-rebind input")
        require(rebound_step.get("scope_widened") is False
                and rebound_step.get("display_fallback_used") is False,
                f"{rebound_step_name} step must preserve application scope")
        if churn_mode == "geometry":
            require(rebound_step.get("target_events") == target_events,
                    "application_geometry_rebound step must expose the ordered target events")
        lifecycle_snapshots = evidence.get("session_snapshots")
        require(isinstance(lifecycle_snapshots, list) and bool(lifecycle_snapshots),
                "application target churn must include session_snapshots")
        terminal_receipts = []
        if isinstance(lifecycle_snapshots, list):
            terminal_receipts = [snapshot.get("terminal_receipt")
                for snapshot in lifecycle_snapshots
                if isinstance(snapshot, dict)
                and snapshot.get("session_id") == session_id
                and snapshot.get("subject_ura") == subject_ura
                and snapshot.get("state") == "closed"
                and isinstance(snapshot.get("terminal_receipt"), dict)]
        require(bool(terminal_receipts),
                "application target churn must expose a closed session terminal receipt")
        if terminal_receipts:
            canonical_receipts = {json.dumps(receipt, sort_keys=True)
                                  for receipt in terminal_receipts}
            require(len(canonical_receipts) == 1,
                    "application target churn terminal receipt must be unique and stable")
            receipt = terminal_receipts[-1]
            require(receipt.get("terminal") is True
                    and receipt.get("receipt_type") == "remoteapp.session.terminal.v1"
                    and receipt.get("terminal_event_type") == "SESSION_CLOSED",
                    "application target churn must expose the canonical terminal receipt")
            require(receipt.get("session_id") == session_id
                    and receipt.get("subject_ura") == subject_ura,
                    "application target churn terminal receipt must bind session and Resource subject")
            require(receipt.get("reason_code") == terminal.get("reason_code"),
                    "application target churn terminal receipt must match the visible terminal reason")
            require(int_field(receipt, "binding_epoch") == binding_after
                    and int_field(receipt, "target_identity_epoch") == identity_after
                    and int_field(receipt, "target_geometry_revision") == geometry_after,
                    "application target churn terminal receipt must bind final target epochs")
            require(int_field(receipt, "terminal_event_sequence") > 0,
                    "application target churn terminal receipt must bind a terminal event sequence")
            if churn_mode == "geometry" and len(geometry_events) == 2:
                require(int_field(receipt, "terminal_event_sequence")
                        > int_field(geometry_events[-1], "sequence"),
                        "application target churn terminal event must follow geometry events")

window_target_churn = evidence.get("window_target_churn")
if window_target_churn is not None:
    require(isinstance(window_target_churn, dict),
            "window_target_churn must be an object when present")
    require(target_kind == "window",
            "window target churn requires selected_target_kind=window")
    require(application_target_churn is None and transport_resume is None
            and terminal_crash_replay is None and target_monitor_worker_recovery is None,
            "window target churn must be a separate lifecycle scenario")
    if isinstance(window_target_churn, dict):
        require(isinstance(target_snapshot, dict)
                and target_snapshot.get("resource_ura") == subject_ura
                and target_snapshot.get("type") == "window"
                and target_metadata.get("capture_target") == "window"
                and int_field(target_metadata, "window_id") > 0,
                "window target snapshot must bind the selected native Window Resource")
        required_window_steps = [
            "media_presented_after_window_geometry_rebind",
            "input_applied_after_window_geometry_rebind",
            "window_geometry_rebound",
        ]
        churn_cursor = step_names.index("input_control_attempted_or_policy_blocked")
        for churn_step in required_window_steps:
            try:
                churn_index = step_names.index(churn_step)
            except ValueError:
                errors.append(f"missing window target churn step: {churn_step}")
                continue
            require(churn_index > churn_cursor,
                    f"window target churn step order is wrong at {churn_step}")
            churn_cursor = churn_index
        require(churn_cursor < step_names.index("session_ended"),
                "window target churn must complete before terminal cleanup")
        require(window_target_churn.get("proof_mode")
                == "real_window_geometry_capture_generation_churn",
                "window target churn must expose capture-generation proof mode")
        require(window_target_churn.get("churn_mode") == "geometry",
                "window target churn mode must be geometry")
        require(window_target_churn.get("session_id") == session_id,
                "window target churn must preserve the created session id")
        require(window_target_churn.get("selected_resource_ura") == subject_ura,
                "window target churn must remain bound to the selected Resource URA")

        binding_before = int_field(window_target_churn, "binding_epoch_before")
        binding_after = int_field(window_target_churn, "binding_epoch_after")
        media_source_before = int_field(window_target_churn, "media_source_epoch_before")
        media_source_after = int_field(window_target_churn, "media_source_epoch_after")
        transport_before = int_field(window_target_churn, "transport_epoch_before")
        transport_after = int_field(window_target_churn, "transport_epoch_after")
        identity_before = int_field(window_target_churn, "target_identity_epoch_before")
        identity_after = int_field(window_target_churn, "target_identity_epoch_after")
        geometry_before = int_field(window_target_churn, "target_geometry_revision_before")
        geometry_after = int_field(window_target_churn, "target_geometry_revision_after")
        verified_before = int_field(window_target_churn, "capture_verified_at_ms_before")
        verified_after = int_field(window_target_churn, "capture_verified_at_ms_after")
        window_before = int_field(window_target_churn, "window_id_before")
        window_after = int_field(window_target_churn, "window_id_after")
        native_width_before = int_field(window_target_churn, "native_width_before")
        native_height_before = int_field(window_target_churn, "native_height_before")
        native_width_after = int_field(window_target_churn, "native_width_after")
        native_height_after = int_field(window_target_churn, "native_height_after")
        presentation_max_width = int_field(window_target_churn, "presentation_max_width")
        presentation_max_height = int_field(window_target_churn, "presentation_max_height")
        frame_width_before = int_field(window_target_churn, "frame_width_before")
        frame_height_before = int_field(window_target_churn, "frame_height_before")
        frame_width_after = int_field(window_target_churn, "frame_width_after")
        frame_height_after = int_field(window_target_churn, "frame_height_after")
        logical_input_width = int_field(window_target_churn, "logical_input_width")
        logical_input_height = int_field(window_target_churn, "logical_input_height")
        require(binding_before > 0 and binding_after > binding_before,
                "window target churn binding epoch must advance")
        require(media_source_before > 0 and media_source_after > media_source_before,
                "window target churn media-source epoch must advance")
        require(transport_before > 0 and transport_after == transport_before,
                "window target churn must preserve one transport epoch")
        require(identity_before > 0 and identity_after == identity_before,
                "window geometry churn must preserve target identity")
        require(geometry_before > 0 and geometry_after > geometry_before,
                "window geometry churn revision must advance")
        require(verified_before > 0 and verified_after > verified_before,
                "window geometry churn capture proof must be reverified")
        require(window_before > 0 and window_after == window_before,
                "window geometry churn must preserve the native window identity")
        require(native_width_before > 0 and native_height_before > 0
                and native_width_after > 0 and native_height_after > 0
                and (native_width_after, native_height_after)
                    != (native_width_before, native_height_before),
                "window geometry churn must prove changed native dimensions")
        require(presentation_max_width > 0 and presentation_max_height > 0
                and window_target_churn.get("presentation_scale_mode") == "native",
                "window geometry churn must expose the bounded native presentation contract")
        expected_before = fit_even_presentation(
            native_width_before, native_height_before,
            presentation_max_width, presentation_max_height)
        expected_after = fit_even_presentation(
            native_width_after, native_height_after,
            presentation_max_width, presentation_max_height)
        require(min(*expected_before, *expected_after) > 0
                and expected_after != expected_before,
                "window churn fixture must produce distinct positive coded presentations")
        require((int_field(window_target_churn, "expected_frame_width_before"),
                 int_field(window_target_churn, "expected_frame_height_before")) == expected_before
                and (frame_width_before, frame_height_before) == expected_before,
                "window initial decoded media must match independently derived FitWithin/even presentation")
        require((int_field(window_target_churn, "expected_frame_width_after"),
                 int_field(window_target_churn, "expected_frame_height_after")) == expected_after
                and (frame_width_after, frame_height_after) == expected_after,
                "window rebound decoded media must match independently derived FitWithin/even presentation")
        require(logical_input_width > 0 and logical_input_height > 0,
                "window rebound must expose positive logical input dimensions")
        require(int_field(window_target_churn, "frames_rendered_after_rebind") > 0,
                "window geometry churn must render media after rebind")
        require(window_target_churn.get("scope_widened") is False
                and window_target_churn.get("display_fallback_used") is False,
                "window geometry churn must preserve WindowSurface scope")

        target_events = window_target_churn.get("target_events")
        target_event_sequences = window_target_churn.get("target_event_sequences")
        require(isinstance(target_events, list)
                and "TARGET_RESIZED" in target_events
                and set(target_events).issubset({"TARGET_MOVED", "TARGET_RESIZED"}),
                "window geometry churn must expose TARGET_RESIZED without unrelated events")
        lifecycle_events = evidence.get("target_lifecycle_events")
        require(isinstance(lifecycle_events, list),
                "window geometry churn must include target_lifecycle_events")
        bound_geometry_events = [event for event in lifecycle_events
            if isinstance(event, dict)
            and event.get("session_id") == session_id
            and event.get("subject_ura") == subject_ura
            and event.get("event_type") in {"TARGET_MOVED", "TARGET_RESIZED"}]
        require(any(event.get("event_type") == "TARGET_RESIZED"
                    for event in bound_geometry_events),
                "window geometry churn must include a bound TARGET_RESIZED event record")
        require([event.get("event_type") for event in bound_geometry_events] == target_events,
                "window geometry lifecycle records must match the event summary")
        sequences = [int_field(event, "sequence") for event in bound_geometry_events]
        require(bool(sequences) and all(sequence > 0 for sequence in sequences)
                and sequences == sorted(set(sequences)),
                "window geometry lifecycle sequences must strictly advance")
        require(target_event_sequences == sequences,
                "window geometry event summary must preserve exact lifecycle sequences")
        for event in bound_geometry_events:
            payload = event.get("payload")
            require(event.get("source_ability") == "remote_desktop.show_session"
                    and event.get("terminal") is False,
                    "window geometry lifecycle event must be non-terminal show_session evidence")
            require(int_field(event, "binding_epoch") == binding_after
                    and int_field(event, "media_source_epoch") == media_source_after
                    and int_field(event, "target_identity_epoch") == identity_after
                    and int_field(event, "target_geometry_revision") == geometry_after
                    and int_field(event, "transport_epoch") == transport_before,
                    "window geometry lifecycle event must bind the committed capture generation")
            require(isinstance(payload, dict),
                    "window geometry lifecycle event payload must be an object")
            if isinstance(payload, dict):
                require(payload.get("subject_ura") == subject_ura
                        and int_field(payload, "previous_binding_epoch") == binding_before
                        and int_field(payload, "previous_media_source_epoch") == media_source_before
                        and int_field(payload, "previous_target_identity_epoch") == identity_before
                        and int_field(payload, "previous_target_geometry_revision") == geometry_before,
                        "window geometry lifecycle payload must bind the previous target generation")

        churn_input = window_target_churn.get("input_after_rebind")
        require(isinstance(churn_input, dict)
                and churn_input.get("result") == "input_applied"
                and int_field(churn_input, "target_geometry_revision") == geometry_after,
                "window geometry churn must apply input against the rebound revision")
        if isinstance(churn_input, dict):
            interactions = churn_input.get("interaction_sequence")
            require(isinstance(interactions, list) and len(interactions) == 4,
                    "window geometry churn input must prove pointer/key down/up")
            if isinstance(interactions, list) and len(interactions) == 4:
                expected = [("pointer", "down"), ("pointer", "up"),
                            ("key", "down"), ("key", "up")]
                interaction_sequences = []
                for index, (interaction, (frame_type, action)) in enumerate(zip(interactions, expected)):
                    prefix = f"window_target_churn.input_after_rebind.interaction_sequence[{index}]"
                    require(isinstance(interaction, dict), f"{prefix} must be an object")
                    if not isinstance(interaction, dict):
                        continue
                    frame = interaction.get("submitted_frame")
                    applied = interaction.get("applied_event")
                    sequence = int_field(interaction, "client_sequence")
                    interaction_sequences.append(sequence)
                    require(isinstance(frame, dict)
                            and frame.get("type") == frame_type
                            and frame.get("action") == action
                            and int_field(frame, "client_sequence") == sequence,
                            f"{prefix} must bind the expected submitted frame")
                    require(isinstance(applied, dict)
                            and applied.get("event_type") in {"INPUT_FRAME_APPLIED", "input_frame_applied"}
                            and applied.get("session_id") == session_id
                            and int_field(applied, "client_sequence") == sequence
                            and int_field(applied, "target_geometry_revision") == geometry_after,
                            f"{prefix} must bind the daemon applied event")
                    require(int_field(interaction, "target_geometry_revision") == geometry_after,
                            f"{prefix} must expose the rebound geometry revision")
                    if isinstance(frame, dict) and frame_type == "pointer":
                        require(int_field(frame, "target_geometry_revision") == geometry_after
                                and int_field(frame, "target_width") == logical_input_width
                                and int_field(frame, "target_height") == logical_input_height,
                                f"{prefix} pointer frame must bind rebound logical dimensions")
                require(all(sequence > 0 for sequence in interaction_sequences)
                        and interaction_sequences == sorted(set(interaction_sequences)),
                        "window geometry churn input sequences must strictly advance")
            input_probe = churn_input.get("input_probe")
            require(isinstance(input_probe, dict)
                    and input_probe.get("source") == "target_center",
                    "window geometry churn input must target the committed window center")
            validate_host_input_effects(
                churn_input,
                "window_target_churn.input_after_rebind",
            )

        execution_snapshots = evidence.get("target_execution_snapshots")
        require(isinstance(execution_snapshots, list) and bool(execution_snapshots),
                "window geometry churn must include target_execution_snapshots")
        initial_snapshot = None
        rebound_snapshot = None
        if isinstance(execution_snapshots, list):
            initial_snapshot = next((snapshot for snapshot in execution_snapshots
                if isinstance(snapshot, dict)
                and snapshot.get("session_id") == session_id
                and snapshot.get("subject_ura") == subject_ura
                and snapshot.get("source_ability") == "remote_desktop.show_session"
                and int_field(snapshot.get("target_binding", {}), "binding_epoch") == binding_before
                and int_field(snapshot.get("target_binding", {}), "media_source_epoch") == media_source_before), None)
            rebound_snapshot = next((snapshot for snapshot in reversed(execution_snapshots)
                if isinstance(snapshot, dict)
                and snapshot.get("session_id") == session_id
                and snapshot.get("subject_ura") == subject_ura
                and snapshot.get("source_ability") == "remote_desktop.show_session"
                and int_field(snapshot.get("target_binding", {}), "binding_epoch") == binding_after
                and int_field(snapshot.get("target_binding", {}), "media_source_epoch") == media_source_after), None)
        require(isinstance(initial_snapshot, dict),
                "window geometry churn must expose the initial capture generation")
        require(isinstance(rebound_snapshot, dict),
                "window geometry churn must expose the rebound capture generation")
        if isinstance(initial_snapshot, dict) and isinstance(rebound_snapshot, dict):
            initial_binding = initial_snapshot.get("target_binding", {})
            rebound_binding = rebound_snapshot.get("target_binding", {})
            initial_proof = initial_binding.get("capture_proof", {})
            rebound_proof = rebound_binding.get("capture_proof", {})
            initial_scope = initial_snapshot.get("scope_audit", {})
            rebound_scope = rebound_snapshot.get("scope_audit", {})
            rebound_bounds = rebound_binding.get("bounds", {})
            require(initial_binding.get("target_kind") == "window"
                    and initial_binding.get("subject_ura") == subject_ura
                    and initial_binding.get("capture_scope") == "WindowSurface"
                    and initial_scope.get("capture_surface") == "WindowSurface"
                    and initial_scope.get("requested_target_kind") == "window"
                    and initial_scope.get("effective_target_kind") == "window"
                    and initial_scope.get("scope_widened") is False
                    and initial_scope.get("display_fallback_used") is False,
                    "window initial snapshot must preserve exact WindowSurface scope")
            require(rebound_binding.get("target_kind") == "window"
                    and rebound_binding.get("subject_ura") == subject_ura
                    and rebound_binding.get("capture_scope") == "WindowSurface"
                    and rebound_scope.get("capture_surface") == "WindowSurface"
                    and rebound_scope.get("requested_target_kind") == "window"
                    and rebound_scope.get("effective_target_kind") == "window"
                    and rebound_scope.get("scope_widened") is False
                    and rebound_scope.get("display_fallback_used") is False,
                    "window rebound snapshot must preserve exact WindowSurface scope")
            require(int_field(initial_binding, "media_source_epoch") == media_source_before
                    and int_field(rebound_binding, "media_source_epoch") == media_source_after,
                    "window snapshots must match media-source generation summary")
            require(int_field(initial_proof, "window_id") == window_before
                    and int_field(rebound_proof, "window_id") == window_after
                    and initial_proof.get("target_kind") == "window"
                    and rebound_proof.get("target_kind") == "window"
                    and initial_proof.get("display_id") is None
                    and rebound_proof.get("display_id") is None
                    and int_field(initial_proof, "verified_at_ms") == verified_before
                    and int_field(rebound_proof, "verified_at_ms") == verified_after,
                    "window snapshots must bind the stable native window and refreshed proof")
            require(int_field(initial_proof, "native_width") == native_width_before
                    and int_field(initial_proof, "native_height") == native_height_before
                    and int_field(rebound_proof, "native_width") == native_width_after
                    and int_field(rebound_proof, "native_height") == native_height_after,
                    "window proofs must match summarized native dimensions")
            selected_platform = target_metadata.get("platform")
            if selected_platform == "linux" and rebound_proof.get("backend") == "xcap":
                require((native_width_after, native_height_after)
                        == (int_field(rebound_bounds, "width"), int_field(rebound_bounds, "height")),
                        "Linux xcap window proof must match exact native bounds, not coded presentation dimensions")

        media_step = step_by_name.get("media_presented_after_window_geometry_rebind", {})
        require(int_field(media_step, "frame_width") == expected_after[0]
                and int_field(media_step, "frame_height") == expected_after[1]
                and int_field(media_step, "expected_frame_width") == expected_after[0]
                and int_field(media_step, "expected_frame_height") == expected_after[1]
                and int_field(media_step, "media_source_epoch") == media_source_after
                and int_field(media_step, "capture_verified_at_ms") == verified_after,
                "window post-rebind media must match the committed capture generation")
        rebound_step = step_by_name.get("window_geometry_rebound", {})
        require(rebound_step.get("session_id") == session_id
                and rebound_step.get("subject_ura") == subject_ura
                and int_field(rebound_step, "binding_epoch_before") == binding_before
                and int_field(rebound_step, "binding_epoch_after") == binding_after
                and int_field(rebound_step, "media_source_epoch_before") == media_source_before
                and int_field(rebound_step, "media_source_epoch_after") == media_source_after
                and int_field(rebound_step, "transport_epoch_before") == transport_before
                and int_field(rebound_step, "transport_epoch_after") == transport_after
                and int_field(rebound_step, "frame_width_before") == expected_before[0]
                and int_field(rebound_step, "frame_height_before") == expected_before[1]
                and int_field(rebound_step, "frame_width_after") == expected_after[0]
                and int_field(rebound_step, "frame_height_after") == expected_after[1]
                and rebound_step.get("target_events") == target_events
                and rebound_step.get("target_event_sequences") == sequences
                and rebound_step.get("input_applied_after_rebind") is True
                and rebound_step.get("scope_widened") is False
                and rebound_step.get("display_fallback_used") is False,
                "window_geometry_rebound step must bind the complete target generation transition")

        lifecycle_snapshots = evidence.get("session_snapshots")
        require(isinstance(lifecycle_snapshots, list) and bool(lifecycle_snapshots),
                "window target churn must include session_snapshots")
        terminal_receipts = []
        if isinstance(lifecycle_snapshots, list):
            terminal_receipts = [snapshot.get("terminal_receipt")
                for snapshot in lifecycle_snapshots
                if isinstance(snapshot, dict)
                and snapshot.get("session_id") == session_id
                and snapshot.get("subject_ura") == subject_ura
                and snapshot.get("state") == "closed"
                and isinstance(snapshot.get("terminal_receipt"), dict)]
        require(bool(terminal_receipts),
                "window target churn must expose a closed session terminal receipt")
        if terminal_receipts:
            canonical_receipts = {json.dumps(receipt, sort_keys=True)
                                  for receipt in terminal_receipts}
            require(len(canonical_receipts) == 1,
                    "window target churn terminal receipt must be unique and stable")
            receipt = terminal_receipts[-1]
            require(receipt.get("terminal") is True
                    and receipt.get("receipt_type") == "remoteapp.session.terminal.v1"
                    and receipt.get("terminal_event_type") == "SESSION_CLOSED"
                    and receipt.get("subject_type") == "window",
                    "window target churn must expose the canonical Window terminal receipt")
            require(receipt.get("session_id") == session_id
                    and receipt.get("subject_ura") == subject_ura
                    and receipt.get("reason_code") == terminal.get("reason_code"),
                    "window target churn terminal receipt must bind session, subject, and reason")
            require(int_field(receipt, "binding_epoch") == binding_after
                    and int_field(receipt, "media_source_epoch") == media_source_after
                    and int_field(receipt, "target_identity_epoch") == identity_after
                    and int_field(receipt, "target_geometry_revision") == geometry_after,
                    "window target churn terminal receipt must bind final target generation")
            require(int_field(receipt, "terminal_event_sequence") > max(sequences, default=0),
                    "window terminal event must follow geometry lifecycle events")

require(terminal.get("reason_code") in {
            "user_cancelled", "caller_ended", "resume_e2e_cleanup",
            "crash_restart_e2e_cleanup",
        },
        "terminal_receipt_visible must expose a known end reason")
require(terminal.get("terminal") is True,
        "terminal_receipt_visible must expose terminal=true")
require(terminal.get("session_id") == session_id,
        "terminal receipt must bind the created session id")

report = {
    "script": "tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh",
    "status": "failed" if errors else "passed",
    "evidence_origin": evidence.get("evidence_origin"),
    "errors": errors,
    "surface": evidence.get("runner_kind"),
    "frontend_url": evidence.get("frontend_url"),
    "session_id": session_id,
    "selected_resource_ura": subject_ura,
    "target_kind": evidence.get("selected_target_kind"),
    "input_result": input_step.get("result"),
    "input_interaction_sequence_verified": (
        input_step.get("result") == "input_applied"
        and isinstance(input_step.get("interaction_sequence"), list)
        and len(input_step["interaction_sequence"]) == 4
    ),
    "host_input_effects_verified": (
        evidence.get("host_input_effects_required") is True
        and isinstance(input_step.get("host_input_effects"), dict)
        and input_step["host_input_effects"].get("exact_target_effect_observed") is True
        and int_field(input_step["host_input_effects"], "unexpected_input_event_count", -1) == 0
        and isinstance(input_step["host_input_effects"].get("observer_baseline"), dict)
        and isinstance(input_step["host_input_effects"].get("observer_final"), dict)
    ),
    "focus_recovery_verified": (
        isinstance(input_step.get("focus_recovery"), dict)
        and input_step["focus_recovery"].get("ability") == "remote_desktop.focus_target"
        and input_step["focus_recovery"].get("invocation_observed") is True
    ),
    "interactive_target_kinds": (
        [evidence.get("selected_target_kind")]
        if input_step.get("result") == "input_applied"
        and evidence.get("selected_target_kind") in {"window", "application"}
        else []
    ),
    "evidence_json": evidence_path,
    **({"transport_resume_summary": transport_resume}
       if isinstance(transport_resume, dict) else {}),
    **({"terminal_crash_replay_summary": terminal_crash_replay}
       if isinstance(terminal_crash_replay, dict) else {}),
    **({"target_monitor_worker_recovery_summary": target_monitor_worker_recovery}
       if isinstance(target_monitor_worker_recovery, dict) else {}),
    **({"application_target_churn_summary": application_target_churn}
       if isinstance(application_target_churn, dict) else {}),
    **({"window_target_churn_summary": window_target_churn}
       if isinstance(window_target_churn, dict) else {}),
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
  python3 "$PROVENANCE_HELPER" project-report --mode "$MODE" \
    --evidence "$EVIDENCE_JSON" --report "$REPORT_JSON"
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
        "visible_label": "pipeline video_only · h264 · bounded_queue_drop_stale_frames · native_media_disabled",
        "media_scope": "video_only",
        "product_ready": False,
        "product_blockers": [
            "native_media_disabled",
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
    "evidence_origin": "contract_self_test",
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
