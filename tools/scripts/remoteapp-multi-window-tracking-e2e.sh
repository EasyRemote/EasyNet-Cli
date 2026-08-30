#!/usr/bin/env bash
# RemoteApp multi-window/application tracking E2E evidence verifier.
#
# Boundary:
# - This harness verifies evidence produced by real host runners for
#   multi-window and multi-application RemoteApp tracking.
# - It does not observe platform windows and does not simulate stream
#   isolation. A live pass requires either --evidence-json from an external
#   runner or --runner-cmd that writes the evidence JSON path provided through
#   EASYNET_REMOTEAPP_MULTI_WINDOW_TRACKING_EVIDENCE_JSON.
# - Self-test validates the evidence contract only; it is not product evidence.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
PROVENANCE_HELPER="$SELF_DIR/remoteapp-evidence-provenance.py"

MODE=skip
OUT_DIR="${EASYNET_REMOTEAPP_MULTI_WINDOW_TRACKING_OUT_DIR:-$REPO_ROOT/target/e2e/remoteapp-multi-window-tracking/$(date -u +%Y%m%d-%H%M%S)-$$}"
RUNNER_CMD="${EASYNET_REMOTEAPP_MULTI_WINDOW_TRACKING_RUNNER_CMD:-}"
EVIDENCE_INPUT="${EASYNET_REMOTEAPP_MULTI_WINDOW_TRACKING_EVIDENCE_JSON:-}"

usage() {
  cat <<'USAGE'
Usage:
  remoteapp-multi-window-tracking-e2e.sh --run --evidence-json PATH
  remoteapp-multi-window-tracking-e2e.sh --run --runner-cmd CMD
  remoteapp-multi-window-tracking-e2e.sh --self-test

Options:
  --run                 Verify real RemoteApp multi-window tracking evidence.
  --self-test           Validate the harness against synthetic positive evidence.
  --runner-cmd CMD      Command that drives real host window churn and writes
                        evidence to EASYNET_REMOTEAPP_MULTI_WINDOW_TRACKING_EVIDENCE_JSON.
  --evidence-json PATH  Existing evidence JSON emitted by a real host runner.
  --out-dir DIR         Report directory.
  -h, --help            Show this help.

Environment:
  EASYNET_REMOTEAPP_MULTI_WINDOW_TRACKING_E2E=1
                        Equivalent to --run.

Evidence contract:
  The evidence JSON must prove real independent tracking, not data-structure
  presence. It must include independent_window_streams, geometry_churn,
  application_window_set_churn, target_loss_rebind, and multi_display_application
  scenarios with public RemoteApp session abilities, selected Resource URA
  subject binding, ordered target lifecycle events, rendered frames, stream
  isolation, decoded-frame probes bound to each selected stream/frame source,
  selected target sentinel binding, no cross-stream sentinel leakage,
  application window-set rebind evidence, and visible terminal receipts.
  Multi-display application may report explicit product unsupported state.

Non-claims:
  A skipped report or self-test does not prove multi-window tracking product
  readiness. This harness verifies one tracking artifact; OS capture, input
  injection, media adaptation, network fallback, frontend Browser/Tauri
  lifecycle, and cross-device product behavior still require their own
  evidence.
USAGE
}

if [[ "${EASYNET_REMOTEAPP_MULTI_WINDOW_TRACKING_E2E:-0}" == "1" ]]; then
  MODE=run
fi

while [[ $# -gt 0 ]]; do
  case "$1" in
    --run) MODE=run; shift ;;
    --self-test) MODE=self-test; shift ;;
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
  python3 - "$REPORT_JSON" "$REPORT_MD" "$status" "$reason" "$EVIDENCE_JSON" <<'PY'
import json
import pathlib
import sys

report_path, md_path, status, reason, evidence_path = sys.argv[1:6]
coverage = {
    "independent_window_streams": False,
    "geometry_churn": False,
    "application_window_set_churn": False,
    "target_loss_rebind": False,
    "multi_display_application": False,
}
report = {
    "script": "tools/scripts/remoteapp-multi-window-tracking-e2e.sh",
    "status": status,
    "reason": reason,
    "evidence_json": evidence_path,
    "coverage": coverage,
    "product_complete_claim": False,
}
pathlib.Path(report_path).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
pathlib.Path(md_path).write_text(
    "# RemoteApp Multi-Window Tracking E2E\n\n"
    f"- Status: `{status}`\n"
    f"- Reason: `{reason}`\n"
    f"- Evidence: `{evidence_path}`\n",
    encoding="utf-8",
)
PY
}

validate_evidence() {
  python3 "$PROVENANCE_HELPER" verify --mode "$MODE" --evidence "$EVIDENCE_JSON"
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

def is_ura(value):
    return isinstance(value, str) and value.startswith("easynet:///")

def integer(value, default=0):
    try:
        return int(value)
    except (TypeError, ValueError):
        return default

required_scenarios = {
    "independent_window_streams",
    "geometry_churn",
    "application_window_set_churn",
    "target_loss_rebind",
    "multi_display_application",
}
required_abilities = (
    "remote_desktop.create_session",
    "remote_desktop.attach",
    "remote_desktop.watch_events",
    "remote_desktop.end_session",
)
terminal_reasons = {"caller_ended", "user_cancelled", "multi_window_tracking_e2e_cleanup"}

require(evidence.get("status") == "passed", "evidence.status must be passed")
require(evidence.get("proof_mode") == "real_multi_window_tracking_matrix",
        "proof_mode must be real_multi_window_tracking_matrix")
require(evidence.get("component_mock") is False, "component_mock must be false")
require(evidence.get("real_backend_runtime") is True, "real_backend_runtime must be true")
require(evidence.get("product_complete_claim") is False,
        "product_complete_claim must remain false")

scenarios = evidence.get("scenarios")
require(isinstance(scenarios, list) and scenarios, "scenarios must be a non-empty list")
scenario_by_name = {}
if isinstance(scenarios, list):
    for scenario in scenarios:
        if not isinstance(scenario, dict):
            errors.append("each scenario entry must be an object")
            continue
        name = scenario.get("scenario")
        if name in scenario_by_name:
            errors.append(f"duplicate scenario entry: {name}")
        scenario_by_name[name] = scenario

missing = sorted(required_scenarios - set(scenario_by_name))
require(not missing, "missing tracking scenarios: " + ", ".join(missing))

scenario_reports = []

def require_session_binding(prefix, scenario):
    subject_ura = scenario.get("selected_resource_ura")
    session_id = scenario.get("session_id")
    require(is_ura(subject_ura), f"{prefix}: selected_resource_ura must be canonical")
    require(isinstance(session_id, str) and session_id, f"{prefix}: session_id must be recorded")
    abilities = scenario.get("abilities")
    require(isinstance(abilities, list) and abilities, f"{prefix}: abilities must be non-empty")
    ability_by_name = {}
    if isinstance(abilities, list):
        for ability in abilities:
            if isinstance(ability, dict) and isinstance(ability.get("name"), str):
                ability_by_name[ability["name"]] = ability
    for ability_name in required_abilities:
        ability = ability_by_name.get(ability_name)
        require(isinstance(ability, dict), f"{prefix}: missing ability {ability_name}")
        if isinstance(ability, dict):
            require(ability.get("subject_ura") == subject_ura,
                    f"{prefix}: {ability_name} must bind selected Resource URA")
            if ability_name != "remote_desktop.create_session":
                require(ability.get("session_id") == session_id,
                        f"{prefix}: {ability_name} must bind session_id")
    return subject_ura, session_id

def require_terminal(prefix, scenario, session_id):
    terminal = scenario.get("terminal_receipt")
    require(isinstance(terminal, dict), f"{prefix}: terminal_receipt must be visible")
    if not isinstance(terminal, dict):
        terminal = {}
    require(terminal.get("terminal") is True,
            f"{prefix}: terminal_receipt.terminal must be true")
    require(terminal.get("session_id") == session_id,
            f"{prefix}: terminal_receipt must bind session_id")
    require(terminal.get("reason_code") in terminal_reasons,
            f"{prefix}: terminal_receipt.reason_code must be a known cleanup/end reason")

for scenario_name in sorted(required_scenarios):
    scenario = scenario_by_name.get(scenario_name)
    if not isinstance(scenario, dict):
        continue
    prefix = scenario_name
    status = scenario.get("status")
    if scenario_name == "multi_display_application" and status == "unsupported":
        require(scenario.get("unsupported_state") == "explicit_product_unsupported",
                f"{prefix}: unsupported_state must be explicit_product_unsupported")
        require(scenario.get("show_unsupported") is True,
                f"{prefix}: show_unsupported must be true")
        require(scenario.get("capture_session_started") is False,
                f"{prefix}: unsupported multi-display app must not start capture session")
        require(scenario.get("MultiAppSurface") is False,
                f"{prefix}: unsupported evidence must say MultiAppSurface is false")
        scenario_reports.append({"scenario": scenario_name, "status": "unsupported"})
        continue

    require(status == "passed", f"{prefix}: status must be passed")
    require(scenario.get("source_only_proof") is False,
            f"{prefix}: source_only_proof must be false")
    require(scenario.get("policy_only") is False,
            f"{prefix}: policy_only must be false")
    subject_ura, session_id = require_session_binding(prefix, scenario)
    require_terminal(prefix, scenario, session_id)

    events = scenario.get("events")
    require(isinstance(events, list) and events, f"{prefix}: events must be non-empty")
    event_types = [event.get("type") for event in events if isinstance(event, dict)]
    revisions = [
        integer(event.get("target_geometry_revision"))
        for event in events
        if isinstance(event, dict) and "target_geometry_revision" in event
    ]

    media = scenario.get("media")
    require(isinstance(media, dict), f"{prefix}: media evidence must be present")
    if not isinstance(media, dict):
        media = {}
    require(integer(media.get("frames_rendered")) > 0,
            f"{prefix}: media.frames_rendered must be positive")
    require(isinstance(media.get("stream_id"), str) and media.get("stream_id"),
            f"{prefix}: media.stream_id must be recorded")
    require(integer(media.get("media_source_epoch")) > 0,
            f"{prefix}: media.media_source_epoch must be positive")

    if scenario_name == "independent_window_streams":
        streams = scenario.get("streams")
        require(isinstance(streams, list) and len(streams) >= 2,
                f"{prefix}: streams must contain at least two concurrent windows")
        stream_ids = set()
        session_ids = set()
        subject_uras = set()
        source_ids = set()
        epochs = set()
        sentinel_ids = set()
        for index, stream in enumerate(streams if isinstance(streams, list) else []):
            stream_prefix = f"{prefix}/streams[{index}]"
            selected_resource_ura = stream.get("selected_resource_ura")
            require(is_ura(stream.get("selected_resource_ura")),
                    f"{stream_prefix}: selected_resource_ura must be canonical")
            require(isinstance(stream.get("session_id"), str) and stream.get("session_id"),
                    f"{stream_prefix}: session_id must be recorded")
            require(isinstance(stream.get("stream_id"), str) and stream.get("stream_id"),
                    f"{stream_prefix}: stream_id must be recorded")
            require(isinstance(stream.get("frame_source_id"), str) and stream.get("frame_source_id"),
                    f"{stream_prefix}: frame_source_id must be recorded")
            require(integer(stream.get("media_source_epoch")) > 0,
                    f"{stream_prefix}: media_source_epoch must be positive")
            require(integer(stream.get("frames_rendered")) > 0,
                    f"{stream_prefix}: frames_rendered must be positive")
            require(stream.get("target_binding_exact") is True,
                    f"{stream_prefix}: target_binding_exact must be true")
            require(isinstance(stream.get("selected_sentinel_id"), str) and stream.get("selected_sentinel_id"),
                    f"{stream_prefix}: selected_sentinel_id must be recorded")
            require(stream.get("sentinel_owner_resource_ura") == selected_resource_ura,
                    f"{stream_prefix}: sentinel_owner_resource_ura must match selected_resource_ura")
            require(stream.get("selected_sentinel_rendered") is True,
                    f"{stream_prefix}: selected_sentinel_rendered must be true")
            require(stream.get("foreign_sentinel_rendered") is False,
                    f"{stream_prefix}: foreign_sentinel_rendered must be false")
            rendered_probe = stream.get("rendered_frame_probe")
            require(isinstance(rendered_probe, dict),
                    f"{stream_prefix}: rendered_frame_probe must be present")
            if not isinstance(rendered_probe, dict):
                rendered_probe = {}
            require(rendered_probe.get("probe_source") == "decoded_frame",
                    f"{stream_prefix}: rendered_frame_probe.probe_source must be decoded_frame")
            require(rendered_probe.get("selected_resource_ura") == selected_resource_ura,
                    f"{stream_prefix}: rendered_frame_probe selected_resource_ura must bind selected stream")
            require(rendered_probe.get("session_id") == stream.get("session_id"),
                    f"{stream_prefix}: rendered_frame_probe session_id must bind stream session")
            require(rendered_probe.get("stream_id") == stream.get("stream_id"),
                    f"{stream_prefix}: rendered_frame_probe stream_id must bind stream")
            require(rendered_probe.get("frame_source_id") == stream.get("frame_source_id"),
                    f"{stream_prefix}: rendered_frame_probe frame_source_id must bind stream frame source")
            require(rendered_probe.get("media_source_epoch") == stream.get("media_source_epoch"),
                    f"{stream_prefix}: rendered_frame_probe media_source_epoch must bind stream")
            require(isinstance(rendered_probe.get("observed_at_ms"), int)
                    and rendered_probe.get("observed_at_ms") > 0,
                    f"{stream_prefix}: rendered_frame_probe observed_at_ms must be recorded")
            require(rendered_probe.get("selected_sentinel_id") == stream.get("selected_sentinel_id"),
                    f"{stream_prefix}: rendered_frame_probe selected_sentinel_id must bind selected sentinel")
            require(isinstance(rendered_probe.get("selected_sentinel_hash"), str)
                    and rendered_probe.get("selected_sentinel_hash"),
                    f"{stream_prefix}: rendered_frame_probe selected_sentinel_hash must be recorded")
            require(rendered_probe.get("selected_sentinel_rendered") is True,
                    f"{stream_prefix}: rendered_frame_probe selected_sentinel_rendered must be true")
            require(rendered_probe.get("foreign_sentinel_rendered") is False,
                    f"{stream_prefix}: rendered_frame_probe foreign_sentinel_rendered must be false")
            stream_ids.add(stream.get("stream_id"))
            session_ids.add(stream.get("session_id"))
            subject_uras.add(stream.get("selected_resource_ura"))
            source_ids.add(stream.get("frame_source_id"))
            epochs.add(stream.get("media_source_epoch"))
            sentinel_ids.add(stream.get("selected_sentinel_id"))
        expected = len(streams) if isinstance(streams, list) else 0
        require(len(stream_ids) == expected, f"{prefix}: stream ids must be distinct")
        require(len(session_ids) == expected, f"{prefix}: session ids must be distinct")
        require(len(subject_uras) == expected, f"{prefix}: selected Resource URAs must be distinct")
        require(len(source_ids) == expected, f"{prefix}: frame source ids must be distinct")
        require(len(epochs) == expected, f"{prefix}: media source epochs must be distinct")
        require(len(sentinel_ids) == expected, f"{prefix}: selected sentinel ids must be distinct")
        require(scenario.get("frames_interleaved") is False,
                f"{prefix}: frames_interleaved must be false")
        require(scenario.get("cross_stream_sentinel_leakage") is False,
                f"{prefix}: cross_stream_sentinel_leakage must be false")

    if scenario_name == "geometry_churn":
        require("TARGET_MOVED" in event_types, f"{prefix}: must include TARGET_MOVED")
        require("TARGET_RESIZED" in event_types, f"{prefix}: must include TARGET_RESIZED")
        require(len(revisions) >= 2 and revisions == sorted(revisions) and len(set(revisions)) == len(revisions),
                f"{prefix}: target_geometry_revision must increase across move/resize")

    if scenario_name == "application_window_set_churn":
        require("APPLICATION_WINDOW_SET_EXPANDED" in event_types
                or "APPLICATION_WINDOW_SET_CONTRACTED" in event_types,
                f"{prefix}: must include application window-set churn")
        require("PENDING_MEDIA_REBIND" in event_types,
                f"{prefix}: must include PENDING_MEDIA_REBIND")
        require("TARGET_REBOUND" in event_types,
                f"{prefix}: must include TARGET_REBOUND")
        require(integer(scenario.get("binding_epoch_after")) > integer(scenario.get("binding_epoch_before")),
                f"{prefix}: binding_epoch_after must increase")
        require(integer(media.get("frames_rendered_after_rebind")) > 0,
                f"{prefix}: media.frames_rendered_after_rebind must be positive")
        require(integer(media.get("committed_window_set_sentinels_rendered_after_rebind")) > 0,
                f"{prefix}: media.committed_window_set_sentinels_rendered_after_rebind must be positive")
        require(media.get("uncommitted_same_app_sentinel_rendered") is False,
                f"{prefix}: media.uncommitted_same_app_sentinel_rendered must be false")
        require(scenario.get("first_display_capture_started") is False,
                f"{prefix}: application churn must not start first-display fallback")
        require(scenario.get("display_fallback_used") is False,
                f"{prefix}: application churn display_fallback_used must be false")

    if scenario_name == "target_loss_rebind":
        require("TARGET_LOST" in event_types, f"{prefix}: must include TARGET_LOST")
        require("TARGET_REBIND_FAILED" in event_types or "TARGET_REBOUND" in event_types,
                f"{prefix}: must include TARGET_REBIND_FAILED or TARGET_REBOUND")
        if "TARGET_REBIND_FAILED" in event_types:
            require(scenario.get("rebind_failure_reason") == "explicit_rebind_required",
                    f"{prefix}: rebind_failure_reason must be explicit_rebind_required")
            require(scenario.get("frontend_action") in {"new_session_required", "retry_session"},
                    f"{prefix}: frontend_action must be actionable")
        if "TARGET_REBOUND" in event_types:
            require(integer(media.get("frames_rendered_after_rebind")) > 0,
                    f"{prefix}: successful rebind must render frames after rebind")
        require(integer(scenario.get("rebind_deadline_ms")) > integer(scenario.get("lost_at_ms")),
                f"{prefix}: rebind_deadline_ms must be after lost_at_ms")

    if scenario_name == "multi_display_application":
        require(scenario.get("MultiAppSurface") is True,
                f"{prefix}: passing multi-display application requires MultiAppSurface")
        require("MULTI_APP_SURFACE_CAPTURE_STARTED" in event_types,
                f"{prefix}: must include MULTI_APP_SURFACE_CAPTURE_STARTED")

    scenario_report = {
        "scenario": scenario_name,
        "status": "passed",
        "session_id": session_id,
        "selected_resource_ura": subject_ura,
        "frames_rendered": integer(media.get("frames_rendered")),
        "events": event_types,
    }
    if scenario_name == "independent_window_streams":
        scenario_report.update({
            "stream_count": expected,
            "distinct_stream_ids": len(stream_ids),
            "distinct_session_ids": len(session_ids),
            "distinct_selected_resource_uras": len(subject_uras),
            "distinct_frame_source_ids": len(source_ids),
            "distinct_media_source_epochs": len(epochs),
            "distinct_selected_sentinel_ids": len(sentinel_ids),
            "frames_interleaved": scenario.get("frames_interleaved"),
            "cross_stream_sentinel_leakage": scenario.get("cross_stream_sentinel_leakage"),
        })
    if scenario_name == "geometry_churn":
        scenario_report["geometry_revision_count"] = len(revisions)
    if scenario_name == "application_window_set_churn":
        scenario_report.update({
            "binding_epoch_before": integer(scenario.get("binding_epoch_before")),
            "binding_epoch_after": integer(scenario.get("binding_epoch_after")),
            "frames_rendered_after_rebind": integer(media.get("frames_rendered_after_rebind")),
            "committed_window_set_sentinels_rendered_after_rebind": integer(
                media.get("committed_window_set_sentinels_rendered_after_rebind")
            ),
            "uncommitted_same_app_sentinel_rendered": media.get("uncommitted_same_app_sentinel_rendered"),
            "first_display_capture_started": scenario.get("first_display_capture_started"),
            "display_fallback_used": scenario.get("display_fallback_used"),
        })
    if scenario_name == "target_loss_rebind":
        scenario_report.update({
            "lost_at_ms": integer(scenario.get("lost_at_ms")),
            "rebind_deadline_ms": integer(scenario.get("rebind_deadline_ms")),
            "rebind_failure_reason": scenario.get("rebind_failure_reason"),
            "frontend_action": scenario.get("frontend_action"),
            "frames_rendered_after_rebind": integer(media.get("frames_rendered_after_rebind")),
        })
    if scenario_name == "multi_display_application":
        scenario_report["MultiAppSurface"] = scenario.get("MultiAppSurface")
    scenario_reports.append(scenario_report)

if errors:
    report = {
        "script": "tools/scripts/remoteapp-multi-window-tracking-e2e.sh",
        "status": "failed",
        "errors": errors,
        "product_complete_claim": False,
    }
    pathlib.Path(report_path).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    pathlib.Path(md_path).write_text(
        "# RemoteApp Multi-Window Tracking E2E\n\n"
        "- Status: `failed`\n"
        + "\n".join(f"- {error}" for error in errors)
        + "\n",
        encoding="utf-8",
    )
    for error in errors:
        print(error, file=sys.stderr)
    raise SystemExit(1)

report = {
    "script": "tools/scripts/remoteapp-multi-window-tracking-e2e.sh",
    "status": "passed",
    "proof_mode": evidence.get("proof_mode"),
    "coverage": {name: name in scenario_by_name for name in sorted(required_scenarios)},
    "scenarios": scenario_reports,
    "product_complete_claim": False,
}
pathlib.Path(report_path).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
pathlib.Path(md_path).write_text(
    "# RemoteApp Multi-Window Tracking E2E\n\n"
    "- Status: `passed`\n"
    "- Proof mode: `real_multi_window_tracking_matrix`\n"
    + "\n".join(f"- {item['scenario']}: `{item['status']}`" for item in scenario_reports)
    + "\n",
    encoding="utf-8",
)
PY
  python3 "$PROVENANCE_HELPER" project-report --mode "$MODE" \
    --evidence "$EVIDENCE_JSON" --report "$REPORT_JSON"
}

write_self_test_evidence() {
  python3 - "$EVIDENCE_JSON" <<'PY'
import json
import pathlib
import sys

def abilities(subject, session_id):
    return [
        {"name": "remote_desktop.create_session", "subject_ura": subject},
        {"name": "remote_desktop.attach", "subject_ura": subject, "session_id": session_id},
        {"name": "remote_desktop.watch_events", "subject_ura": subject, "session_id": session_id},
        {"name": "remote_desktop.end_session", "subject_ura": subject, "session_id": session_id},
    ]

def base(name, subject, session_id):
    return {
        "scenario": name,
        "status": "passed",
        "source_only_proof": False,
        "policy_only": False,
        "selected_resource_ura": subject,
        "session_id": session_id,
        "abilities": abilities(subject, session_id),
        "media": {
            "stream_id": f"stream-{name}",
            "media_source_epoch": 10,
            "frames_rendered": 120,
        },
        "events": [{"type": "TARGET_STABLE", "target_geometry_revision": 1}],
        "terminal_receipt": {
            "terminal": True,
            "session_id": session_id,
            "reason_code": "multi_window_tracking_e2e_cleanup",
        },
    }

independent = base(
    "independent_window_streams",
    "easynet:///r/acme/resource/device.dev/window.a",
    "sess-independent",
)
independent["streams"] = [
    {
        "selected_resource_ura": "easynet:///r/acme/resource/device.dev/window.a",
        "session_id": "sess-window-a",
        "stream_id": "stream-window-a",
        "frame_source_id": "cg-window-a",
        "media_source_epoch": 11,
        "frames_rendered": 90,
        "target_binding_exact": True,
        "selected_sentinel_id": "sentinel-window-a",
        "sentinel_owner_resource_ura": "easynet:///r/acme/resource/device.dev/window.a",
        "selected_sentinel_rendered": True,
        "foreign_sentinel_rendered": False,
        "rendered_frame_probe": {
            "probe_source": "decoded_frame",
            "selected_resource_ura": "easynet:///r/acme/resource/device.dev/window.a",
            "session_id": "sess-window-a",
            "stream_id": "stream-window-a",
            "frame_source_id": "cg-window-a",
            "media_source_epoch": 11,
            "observed_at_ms": 1787335001000,
            "selected_sentinel_id": "sentinel-window-a",
            "selected_sentinel_hash": "sha256:sentinel-window-a",
            "selected_sentinel_rendered": True,
            "foreign_sentinel_rendered": False,
        },
    },
    {
        "selected_resource_ura": "easynet:///r/acme/resource/device.dev/window.b",
        "session_id": "sess-window-b",
        "stream_id": "stream-window-b",
        "frame_source_id": "cg-window-b",
        "media_source_epoch": 12,
        "frames_rendered": 88,
        "target_binding_exact": True,
        "selected_sentinel_id": "sentinel-window-b",
        "sentinel_owner_resource_ura": "easynet:///r/acme/resource/device.dev/window.b",
        "selected_sentinel_rendered": True,
        "foreign_sentinel_rendered": False,
        "rendered_frame_probe": {
            "probe_source": "decoded_frame",
            "selected_resource_ura": "easynet:///r/acme/resource/device.dev/window.b",
            "session_id": "sess-window-b",
            "stream_id": "stream-window-b",
            "frame_source_id": "cg-window-b",
            "media_source_epoch": 12,
            "observed_at_ms": 1787335001100,
            "selected_sentinel_id": "sentinel-window-b",
            "selected_sentinel_hash": "sha256:sentinel-window-b",
            "selected_sentinel_rendered": True,
            "foreign_sentinel_rendered": False,
        },
    },
]
independent["frames_interleaved"] = False
independent["cross_stream_sentinel_leakage"] = False

geometry = base(
    "geometry_churn",
    "easynet:///r/acme/resource/device.dev/window.geometry",
    "sess-geometry",
)
geometry["events"] = [
    {"type": "TARGET_MOVED", "target_geometry_revision": 2},
    {"type": "TARGET_RESIZED", "target_geometry_revision": 3},
]

app = base(
    "application_window_set_churn",
    "easynet:///r/acme/resource/device.dev/application.editor",
    "sess-app",
)
app["events"] = [
    {"type": "APPLICATION_WINDOW_SET_EXPANDED", "target_geometry_revision": 2},
    {"type": "PENDING_MEDIA_REBIND", "target_geometry_revision": 3},
    {"type": "TARGET_REBOUND", "target_geometry_revision": 4},
]
app["binding_epoch_before"] = 1
app["binding_epoch_after"] = 2
app["first_display_capture_started"] = False
app["display_fallback_used"] = False
app["media"]["frames_rendered_after_rebind"] = 45
app["media"]["committed_window_set_sentinels_rendered_after_rebind"] = 2
app["media"]["uncommitted_same_app_sentinel_rendered"] = False

loss = base(
    "target_loss_rebind",
    "easynet:///r/acme/resource/device.dev/window.loss",
    "sess-loss",
)
loss["events"] = [
    {"type": "TARGET_LOST", "target_geometry_revision": 2},
    {"type": "TARGET_REBIND_FAILED", "target_geometry_revision": 2},
]
loss["lost_at_ms"] = 1000
loss["rebind_deadline_ms"] = 31000
loss["rebind_failure_reason"] = "explicit_rebind_required"
loss["frontend_action"] = "new_session_required"

multi = {
    "scenario": "multi_display_application",
    "status": "unsupported",
    "unsupported_state": "explicit_product_unsupported",
    "show_unsupported": True,
    "capture_session_started": False,
    "MultiAppSurface": False,
}

evidence = {
    "status": "passed",
    "evidence_origin": "contract_self_test",
    "proof_mode": "real_multi_window_tracking_matrix",
    "component_mock": False,
    "real_backend_runtime": True,
    "product_complete_claim": False,
    "scenarios": [independent, geometry, app, loss, multi],
}
pathlib.Path(sys.argv[1]).write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
}

case "$MODE" in
  skip)
    write_report "skipped" "set EASYNET_REMOTEAPP_MULTI_WINDOW_TRACKING_E2E=1 or pass --run with real tracking evidence"
    echo "remoteapp-multi-window-tracking-e2e skipped; report: $REPORT_JSON"
    ;;
  self-test)
    write_self_test_evidence
    validate_evidence
    echo "remoteapp-multi-window-tracking-e2e self-test ok"
    ;;
  run)
    if [[ -n "$RUNNER_CMD" ]]; then
      EASYNET_REMOTEAPP_MULTI_WINDOW_TRACKING_EVIDENCE_JSON="$EVIDENCE_JSON" \
        bash -lc "$RUNNER_CMD" >"$RUNNER_STDOUT" 2>"$RUNNER_STDERR"
    elif [[ -n "$EVIDENCE_INPUT" ]]; then
      cp "$EVIDENCE_INPUT" "$EVIDENCE_JSON"
    else
      write_report "failed" "run mode requires --evidence-json or --runner-cmd"
      echo "remoteapp-multi-window-tracking-e2e failed; report: $REPORT_JSON" >&2
      exit 64
    fi
    validate_evidence
    echo "remoteapp-multi-window-tracking-e2e passed; report: $REPORT_JSON"
    ;;
  *)
    echo "unknown mode: $MODE" >&2
    exit 64
    ;;
esac
