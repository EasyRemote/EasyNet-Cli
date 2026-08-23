#!/usr/bin/env bash
# RemoteApp crash/restart recovery E2E evidence verifier.
#
# Boundary:
# - This harness verifies evidence produced by real daemon/plugin crash or
#   restart runners.
# - It does not kill processes or simulate crash recovery itself. A live pass
#   requires either --evidence-json from an external runner or --runner-cmd that
#   writes the evidence JSON path provided through
#   EASYNET_REMOTEAPP_CRASH_RESTART_RECOVERY_EVIDENCE_JSON.
# - Self-test validates the evidence contract only; it is not product evidence.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"

MODE=skip
OUT_DIR="${EASYNET_REMOTEAPP_CRASH_RESTART_RECOVERY_OUT_DIR:-$REPO_ROOT/target/e2e/remoteapp-crash-restart-recovery/$(date -u +%Y%m%d-%H%M%S)-$$}"
RUNNER_CMD="${EASYNET_REMOTEAPP_CRASH_RESTART_RECOVERY_RUNNER_CMD:-}"
EVIDENCE_INPUT="${EASYNET_REMOTEAPP_CRASH_RESTART_RECOVERY_EVIDENCE_JSON:-}"

usage() {
  cat <<'USAGE'
Usage:
  remoteapp-crash-restart-recovery-e2e.sh --run --evidence-json PATH
  remoteapp-crash-restart-recovery-e2e.sh --run --runner-cmd CMD
  remoteapp-crash-restart-recovery-e2e.sh --self-test

Options:
  --run                 Verify real RemoteApp crash/restart recovery evidence.
  --self-test           Validate the harness against synthetic positive evidence.
  --runner-cmd CMD      Command that drives real crash/restart scenarios and writes
                        evidence to EASYNET_REMOTEAPP_CRASH_RESTART_RECOVERY_EVIDENCE_JSON.
  --evidence-json PATH  Existing evidence JSON emitted by a real recovery runner.
  --out-dir DIR         Report directory.
  -h, --help            Show this help.

Environment:
  EASYNET_REMOTEAPP_CRASH_RESTART_RECOVERY_E2E=1
                        Equivalent to --run.

Evidence contract:
  The evidence JSON must prove a real crash/restart recovery matrix, not
  source-only recovery hooks. Required scenarios are daemon_restart_active_session,
  plugin_worker_restart, terminal_receipt_replay_after_crash, and
  stale_socket_restart_cleanup. Evidence must prove public RemoteApp ability
  paths, same-session recovery or same terminal receipt replay, recovered replay
  guards/idempotency, ordered lifecycle events, watch/media reattachment,
  endpoint readiness, post-reattach rendered frames, and visible terminal
  receipts.

Non-claims:
  A skipped report or self-test does not prove crash/restart product readiness.
  This harness verifies one recovery artifact; OS capture, input injection,
  media adaptation, network fallback, frontend Browser/Tauri lifecycle, and
  cross-device product behavior still require their own evidence.
USAGE
}

if [[ "${EASYNET_REMOTEAPP_CRASH_RESTART_RECOVERY_E2E:-0}" == "1" ]]; then
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
    "daemon_restart_active_session": False,
    "plugin_worker_restart": False,
    "terminal_receipt_replay_after_crash": False,
    "stale_socket_restart_cleanup": False,
}
report = {
    "script": "tools/scripts/remoteapp-crash-restart-recovery-e2e.sh",
    "status": status,
    "reason": reason,
    "evidence_json": evidence_path,
    "coverage": coverage,
    "product_complete_claim": False,
}
pathlib.Path(report_path).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
pathlib.Path(md_path).write_text(
    "# RemoteApp Crash/Restart Recovery E2E\n\n"
    f"- Status: `{status}`\n"
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

def is_ura(value):
    return isinstance(value, str) and value.startswith("easynet:///")

def integer(value, default=0):
    try:
        return int(value)
    except (TypeError, ValueError):
        return default

def find_event_at(event_times, event_type):
    values = event_times.get(event_type) or []
    return values[0] if values else 0

required_scenarios = {
    "daemon_restart_active_session",
    "plugin_worker_restart",
    "terminal_receipt_replay_after_crash",
    "stale_socket_restart_cleanup",
}
required_abilities = (
    "remote_desktop.create_session",
    "remote_desktop.show_session",
    "remote_desktop.watch_events",
    "remote_desktop.end_session",
)
terminal_reasons = {
    "caller_ended",
    "user_cancelled",
    "crash_restart_e2e_cleanup",
    "session_recovered_then_closed",
}

require(evidence.get("status") == "passed", "evidence.status must be passed")
require(evidence.get("proof_mode") == "real_crash_restart_recovery_matrix",
        "proof_mode must be real_crash_restart_recovery_matrix")
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
require(not missing, "missing recovery scenarios: " + ", ".join(missing))

def require_common(prefix, scenario):
    require(scenario.get("status") == "passed", f"{prefix}: status must be passed")
    require(scenario.get("source_only_proof") is False,
            f"{prefix}: source_only_proof must be false")
    require(scenario.get("policy_only") is False,
            f"{prefix}: policy_only must be false")
    subject_ura = scenario.get("selected_resource_ura")
    session_id = scenario.get("session_id")
    require(is_ura(subject_ura), f"{prefix}: selected_resource_ura must be canonical")
    require(isinstance(session_id, str) and session_id, f"{prefix}: session_id must be recorded")
    require(isinstance(scenario.get("descriptor_version"), str) and scenario.get("descriptor_version"),
            f"{prefix}: descriptor_version must be recorded")
    scenario_started_at_ms = integer(scenario.get("scenario_started_at_ms"))
    require(scenario_started_at_ms > 0,
            f"{prefix}: scenario_started_at_ms must be recorded")
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
    return subject_ura, session_id, scenario_started_at_ms

def require_recovery_guards(prefix, recovery):
    require(isinstance(recovery, dict), f"{prefix}: recovery evidence must be present")
    if not isinstance(recovery, dict):
        recovery = {}
    require(recovery.get("wal_replayed") is True,
            f"{prefix}: recovery.wal_replayed must be true")
    require(recovery.get("idempotency_state_recovered") is True,
            f"{prefix}: idempotency_state_recovered must be true")
    require(recovery.get("replay_guard_recovered") is True,
            f"{prefix}: replay_guard_recovered must be true")
    require(recovery.get("lock_owner_recovered") is True,
            f"{prefix}: lock_owner_recovered must be true")
    require(recovery.get("duplicate_invocation_replayed") is False,
            f"{prefix}: duplicate_invocation_replayed must be false")
    require(integer(recovery.get("restart_epoch_after")) > integer(recovery.get("restart_epoch_before")),
            f"{prefix}: restart_epoch_after must increase")

def require_terminal(prefix, terminal, session_id):
    require(isinstance(terminal, dict), f"{prefix}: terminal_receipt must be visible")
    if not isinstance(terminal, dict):
        terminal = {}
    require(terminal.get("terminal") is True,
            f"{prefix}: terminal_receipt.terminal must be true")
    require(terminal.get("session_id") == session_id,
            f"{prefix}: terminal_receipt must bind session_id")
    require(terminal.get("reason_code") in terminal_reasons,
            f"{prefix}: terminal_receipt.reason_code must be a known cleanup/end reason")
    require(isinstance(terminal.get("receipt_id"), str) and terminal.get("receipt_id"),
            f"{prefix}: terminal_receipt.receipt_id must be recorded")

scenario_reports = []
def recovery_summary(recovery):
    if not isinstance(recovery, dict):
        recovery = {}
    return {
        "wal_replayed": recovery.get("wal_replayed"),
        "idempotency_state_recovered": recovery.get("idempotency_state_recovered"),
        "replay_guard_recovered": recovery.get("replay_guard_recovered"),
        "lock_owner_recovered": recovery.get("lock_owner_recovered"),
        "duplicate_invocation_replayed": recovery.get("duplicate_invocation_replayed"),
        "restart_epoch_before": recovery.get("restart_epoch_before"),
        "restart_epoch_after": recovery.get("restart_epoch_after"),
    }

def terminal_visible(terminal, session_id):
    return (
        isinstance(terminal, dict)
        and terminal.get("terminal") is True
        and terminal.get("session_id") == session_id
        and isinstance(terminal.get("receipt_id"), str)
        and bool(terminal.get("receipt_id"))
    )

for scenario_name in sorted(required_scenarios):
    scenario = scenario_by_name.get(scenario_name)
    if not isinstance(scenario, dict):
        continue
    prefix = scenario_name
    subject_ura, session_id, scenario_started_at_ms = require_common(prefix, scenario)
    events = scenario.get("events")
    require(isinstance(events, list) and events, f"{prefix}: events must be non-empty")
    event_types = []
    event_times = {}
    last_event_at_ms = 0
    if isinstance(events, list):
        for index, event in enumerate(events):
            if not isinstance(event, dict):
                errors.append(f"{prefix}: events[{index}] must be an object")
                continue
            event_prefix = f"{prefix}: events[{index}]"
            event_type = event.get("type")
            event_at_ms = integer(event.get("at_ms"))
            event_types.append(event_type)
            event_times.setdefault(event_type, []).append(event_at_ms)
            require(event_at_ms >= scenario_started_at_ms,
                    f"{event_prefix}.at_ms must be at or after scenario_started_at_ms")
            require(event_at_ms > last_event_at_ms,
                    f"{prefix}: events must be strictly ordered by at_ms")
            last_event_at_ms = max(last_event_at_ms, event_at_ms)
            require(event.get("selected_resource_ura") == subject_ura,
                    f"{event_prefix}.selected_resource_ura must bind selected Resource URA")
            require(event.get("session_id") == session_id,
                    f"{event_prefix}.session_id must bind session_id")

    def require_event_order(first, second, message):
        require(find_event_at(event_times, first) > 0 and find_event_at(event_times, second) > 0
                and find_event_at(event_times, first) < find_event_at(event_times, second),
                f"{prefix}: {message}")

    if scenario_name == "daemon_restart_active_session":
        require("PROCESS_STOPPED_UNCLEAN" in event_types,
                f"{prefix}: must include PROCESS_STOPPED_UNCLEAN")
        require("DAEMON_RESTARTED" in event_types,
                f"{prefix}: must include DAEMON_RESTARTED")
        require("SESSION_REHYDRATED" in event_types,
                f"{prefix}: must include SESSION_REHYDRATED")
        require_event_order("PROCESS_STOPPED_UNCLEAN", "DAEMON_RESTARTED",
                            "PROCESS_STOPPED_UNCLEAN must occur before DAEMON_RESTARTED")
        require_event_order("DAEMON_RESTARTED", "SESSION_REHYDRATED",
                            "DAEMON_RESTARTED must occur before SESSION_REHYDRATED")
        require_recovery_guards(prefix, scenario.get("recovery"))
        before = scenario.get("before_restart")
        after = scenario.get("after_restart")
        require(isinstance(before, dict) and isinstance(after, dict),
                f"{prefix}: before_restart and after_restart must be present")
        if not isinstance(before, dict):
            before = {}
        if not isinstance(after, dict):
            after = {}
        for key in ("session_id", "selected_resource_ura", "descriptor_version", "target_binding_epoch", "transport_epoch"):
            require(before.get(key) == after.get(key),
                    f"{prefix}: {key} must remain stable across restart")
        require(after.get("session_state") == "active",
                f"{prefix}: after_restart.session_state must be active")
        require(after.get("show_session_public") is True,
                f"{prefix}: show_session after restart must use public ability")
        show_session_observed_at_ms = integer(after.get("show_session_observed_at_ms"))
        require(show_session_observed_at_ms > find_event_at(event_times, "DAEMON_RESTARTED"),
                f"{prefix}: show_session_observed_at_ms must be after DAEMON_RESTARTED")
        require(after.get("watch_events_reattached") is True,
                f"{prefix}: watch_events_reattached must be true")
        watch_reattached_at_ms = integer(after.get("watch_events_reattached_at_ms"))
        require(watch_reattached_at_ms > find_event_at(event_times, "SESSION_REHYDRATED"),
                f"{prefix}: watch_events_reattached_at_ms must be after SESSION_REHYDRATED")
        require(after.get("media_reattached") is True,
                f"{prefix}: media_reattached must be true")
        media_reattached_at_ms = integer(after.get("media_reattached_at_ms"))
        require(media_reattached_at_ms > watch_reattached_at_ms,
                f"{prefix}: media_reattached_at_ms must be after watch_events_reattached_at_ms")
        require(integer(after.get("frames_rendered_after_restart")) > 0,
                f"{prefix}: frames_rendered_after_restart must be positive")
        require(integer(after.get("first_frame_rendered_after_restart_at_ms")) > media_reattached_at_ms,
                f"{prefix}: first_frame_rendered_after_restart_at_ms must be after media_reattached_at_ms")
        require_recovery_guards(prefix, scenario.get("recovery"))
        require_terminal(prefix, scenario.get("terminal_receipt"), session_id)
        scenario_reports.append({
            "scenario": scenario_name,
            "status": scenario.get("status"),
            "selected_resource_ura": subject_ura,
            "session_id": session_id,
            "descriptor_version": scenario.get("descriptor_version"),
            "events": event_types,
            "recovery": recovery_summary(scenario.get("recovery")),
            "same_session_after_restart": (
                isinstance(before, dict)
                and isinstance(after, dict)
                and before.get("session_id") == after.get("session_id") == session_id
                and before.get("selected_resource_ura") == after.get("selected_resource_ura") == subject_ura
                and before.get("descriptor_version") == after.get("descriptor_version")
                and before.get("target_binding_epoch") == after.get("target_binding_epoch")
                and before.get("transport_epoch") == after.get("transport_epoch")
            ),
            "session_state_after_restart": after.get("session_state") if isinstance(after, dict) else None,
            "watch_events_reattached": after.get("watch_events_reattached") if isinstance(after, dict) else None,
            "media_reattached": after.get("media_reattached") if isinstance(after, dict) else None,
            "frames_rendered_after_restart": after.get("frames_rendered_after_restart") if isinstance(after, dict) else None,
            "terminal_receipt_visible": terminal_visible(scenario.get("terminal_receipt"), session_id),
        })

    if scenario_name == "plugin_worker_restart":
        require("PLUGIN_WORKER_CRASHED" in event_types,
                f"{prefix}: must include PLUGIN_WORKER_CRASHED")
        require("PLUGIN_WORKER_RESTARTED" in event_types,
                f"{prefix}: must include PLUGIN_WORKER_RESTARTED")
        require("TARGET_MONITOR_RESTARTED" in event_types,
                f"{prefix}: must include TARGET_MONITOR_RESTARTED")
        require_event_order("PLUGIN_WORKER_CRASHED", "PLUGIN_WORKER_RESTARTED",
                            "PLUGIN_WORKER_CRASHED must occur before PLUGIN_WORKER_RESTARTED")
        require_event_order("PLUGIN_WORKER_RESTARTED", "TARGET_MONITOR_RESTARTED",
                            "PLUGIN_WORKER_RESTARTED must occur before TARGET_MONITOR_RESTARTED")
        require(scenario.get("same_public_session") is True,
                f"{prefix}: same_public_session must be true")
        require(integer(scenario.get("media_source_epoch_after")) > integer(scenario.get("media_source_epoch_before")),
                f"{prefix}: media_source_epoch_after must increase")
        require(integer(scenario.get("frames_rendered_after_worker_restart")) > 0,
                f"{prefix}: frames_rendered_after_worker_restart must be positive")
        require(integer(scenario.get("first_frame_rendered_after_worker_restart_at_ms"))
                > find_event_at(event_times, "TARGET_MONITOR_RESTARTED"),
                f"{prefix}: first_frame_rendered_after_worker_restart_at_ms must be after TARGET_MONITOR_RESTARTED")
        require(scenario.get("new_consent_required") is False,
                f"{prefix}: plugin restart must not mint new consent")
        require_recovery_guards(prefix, scenario.get("recovery"))
        require_terminal(prefix, scenario.get("terminal_receipt"), session_id)
        scenario_reports.append({
            "scenario": scenario_name,
            "status": scenario.get("status"),
            "selected_resource_ura": subject_ura,
            "session_id": session_id,
            "descriptor_version": scenario.get("descriptor_version"),
            "events": event_types,
            "recovery": recovery_summary(scenario.get("recovery")),
            "same_public_session": scenario.get("same_public_session"),
            "media_source_epoch_increased": (
                integer(scenario.get("media_source_epoch_after"))
                > integer(scenario.get("media_source_epoch_before"))
            ),
            "frames_rendered_after_worker_restart": scenario.get("frames_rendered_after_worker_restart"),
            "new_consent_required": scenario.get("new_consent_required"),
            "terminal_receipt_visible": terminal_visible(scenario.get("terminal_receipt"), session_id),
        })

    if scenario_name == "terminal_receipt_replay_after_crash":
        require("END_SESSION_ACCEPTED" in event_types,
                f"{prefix}: must include END_SESSION_ACCEPTED")
        require("PROCESS_STOPPED_UNCLEAN" in event_types,
                f"{prefix}: must include PROCESS_STOPPED_UNCLEAN")
        require("TERMINAL_RECEIPT_REPLAYED" in event_types,
                f"{prefix}: must include TERMINAL_RECEIPT_REPLAYED")
        require_event_order("END_SESSION_ACCEPTED", "PROCESS_STOPPED_UNCLEAN",
                            "END_SESSION_ACCEPTED must occur before PROCESS_STOPPED_UNCLEAN")
        require_event_order("PROCESS_STOPPED_UNCLEAN", "TERMINAL_RECEIPT_REPLAYED",
                            "PROCESS_STOPPED_UNCLEAN must occur before TERMINAL_RECEIPT_REPLAYED")
        receipt_before = scenario.get("terminal_receipt_before_crash")
        receipt_after = scenario.get("terminal_receipt_after_restart")
        require(isinstance(receipt_before, dict) and isinstance(receipt_after, dict),
                f"{prefix}: terminal receipts before and after restart must be present")
        if not isinstance(receipt_before, dict):
            receipt_before = {}
        if not isinstance(receipt_after, dict):
            receipt_after = {}
        require(receipt_before.get("receipt_id") == receipt_after.get("receipt_id"),
                f"{prefix}: terminal receipt id must be replayed")
        require(receipt_after.get("terminal") is True,
                f"{prefix}: replayed receipt must be terminal")
        require(scenario.get("repeat_end_session_idempotent") is True,
                f"{prefix}: repeat_end_session_idempotent must be true")
        require(scenario.get("show_session_after_restart_state") == "closed",
                f"{prefix}: show_session_after_restart_state must be closed")
        require(integer(scenario.get("show_session_after_restart_observed_at_ms"))
                > find_event_at(event_times, "TERMINAL_RECEIPT_REPLAYED"),
                f"{prefix}: show_session_after_restart_observed_at_ms must be after TERMINAL_RECEIPT_REPLAYED")
        require_recovery_guards(prefix, scenario.get("recovery"))
        require_terminal(prefix, receipt_after, session_id)
        scenario_reports.append({
            "scenario": scenario_name,
            "status": scenario.get("status"),
            "selected_resource_ura": subject_ura,
            "session_id": session_id,
            "descriptor_version": scenario.get("descriptor_version"),
            "events": event_types,
            "recovery": recovery_summary(scenario.get("recovery")),
            "terminal_receipt_replayed": (
                isinstance(receipt_before, dict)
                and isinstance(receipt_after, dict)
                and receipt_before.get("receipt_id") == receipt_after.get("receipt_id")
            ),
            "repeat_end_session_idempotent": scenario.get("repeat_end_session_idempotent"),
            "show_session_after_restart_state": scenario.get("show_session_after_restart_state"),
            "terminal_receipt_visible": terminal_visible(receipt_after, session_id),
        })

    if scenario_name == "stale_socket_restart_cleanup":
        require("STALE_CONTROL_SOCKET_DETECTED" in event_types,
                f"{prefix}: must include STALE_CONTROL_SOCKET_DETECTED")
        require("STALE_INVOCATION_SOCKET_DETECTED" in event_types,
                f"{prefix}: must include STALE_INVOCATION_SOCKET_DETECTED")
        require("DAEMON_READY_AFTER_RESTART" in event_types,
                f"{prefix}: must include DAEMON_READY_AFTER_RESTART")
        require_event_order("STALE_CONTROL_SOCKET_DETECTED", "DAEMON_READY_AFTER_RESTART",
                            "STALE_CONTROL_SOCKET_DETECTED must occur before DAEMON_READY_AFTER_RESTART")
        require_event_order("STALE_INVOCATION_SOCKET_DETECTED", "DAEMON_READY_AFTER_RESTART",
                            "STALE_INVOCATION_SOCKET_DETECTED must occur before DAEMON_READY_AFTER_RESTART")
        require(scenario.get("control_endpoint_ready") is True,
                f"{prefix}: control_endpoint_ready must be true")
        require(scenario.get("invocation_endpoint_ready") is True,
                f"{prefix}: invocation_endpoint_ready must be true")
        endpoint_ready_at_ms = integer(scenario.get("endpoint_ready_at_ms"))
        require(endpoint_ready_at_ms > find_event_at(event_times, "DAEMON_READY_AFTER_RESTART"),
                f"{prefix}: endpoint_ready_at_ms must be after DAEMON_READY_AFTER_RESTART")
        require(scenario.get("stale_socket_cleanup_explicit") is True,
                f"{prefix}: stale_socket_cleanup_explicit must be true")
        require(scenario.get("manual_cleanup_required") is False,
                f"{prefix}: manual_cleanup_required must be false")
        require_recovery_guards(prefix, scenario.get("recovery"))
        require_terminal(prefix, scenario.get("terminal_receipt"), session_id)
        scenario_reports.append({
            "scenario": scenario_name,
            "status": scenario.get("status"),
            "selected_resource_ura": subject_ura,
            "session_id": session_id,
            "descriptor_version": scenario.get("descriptor_version"),
            "events": event_types,
            "recovery": recovery_summary(scenario.get("recovery")),
            "control_endpoint_ready": scenario.get("control_endpoint_ready"),
            "invocation_endpoint_ready": scenario.get("invocation_endpoint_ready"),
            "stale_socket_cleanup_explicit": scenario.get("stale_socket_cleanup_explicit"),
            "manual_cleanup_required": scenario.get("manual_cleanup_required"),
            "terminal_receipt_visible": terminal_visible(scenario.get("terminal_receipt"), session_id),
        })

if errors:
    report = {
        "script": "tools/scripts/remoteapp-crash-restart-recovery-e2e.sh",
        "status": "failed",
        "errors": errors,
        "product_complete_claim": False,
    }
    pathlib.Path(report_path).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    pathlib.Path(md_path).write_text(
        "# RemoteApp Crash/Restart Recovery E2E\n\n"
        "- Status: `failed`\n"
        + "\n".join(f"- {error}" for error in errors)
        + "\n",
        encoding="utf-8",
    )
    for error in errors:
        print(error, file=sys.stderr)
    raise SystemExit(1)

report = {
    "script": "tools/scripts/remoteapp-crash-restart-recovery-e2e.sh",
    "status": "passed",
    "proof_mode": evidence.get("proof_mode"),
    "coverage": {name: name in scenario_by_name for name in sorted(required_scenarios)},
    "scenarios": scenario_reports,
    "product_complete_claim": False,
}
pathlib.Path(report_path).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
pathlib.Path(md_path).write_text(
    "# RemoteApp Crash/Restart Recovery E2E\n\n"
    "- Status: `passed`\n"
    "- Proof mode: `real_crash_restart_recovery_matrix`\n"
    + "\n".join(f"- {item['scenario']}: `{item['status']}`" for item in scenario_reports)
    + "\n",
    encoding="utf-8",
)
PY
}

write_self_test_evidence() {
  python3 - "$EVIDENCE_JSON" <<'PY'
import json
import pathlib
import sys

def abilities(subject, session_id):
    return [
        {"name": "remote_desktop.create_session", "subject_ura": subject},
        {"name": "remote_desktop.show_session", "subject_ura": subject, "session_id": session_id},
        {"name": "remote_desktop.watch_events", "subject_ura": subject, "session_id": session_id},
        {"name": "remote_desktop.end_session", "subject_ura": subject, "session_id": session_id},
    ]

def recovery(before=1, after=2):
    return {
        "wal_replayed": True,
        "idempotency_state_recovered": True,
        "replay_guard_recovered": True,
        "lock_owner_recovered": True,
        "duplicate_invocation_replayed": False,
        "restart_epoch_before": before,
        "restart_epoch_after": after,
    }

def terminal(session_id, receipt_id):
    return {
        "terminal": True,
        "session_id": session_id,
        "reason_code": "crash_restart_e2e_cleanup",
        "receipt_id": receipt_id,
    }

def common(name, subject, session_id):
    return {
        "scenario": name,
        "status": "passed",
        "source_only_proof": False,
        "policy_only": False,
        "selected_resource_ura": subject,
        "session_id": session_id,
        "descriptor_version": "1.0.0",
        "scenario_started_at_ms": 1787333000000,
        "abilities": abilities(subject, session_id),
    }

def event(event_type, subject, session_id, offset_ms):
    return {
        "type": event_type,
        "at_ms": 1787333000000 + offset_ms,
        "selected_resource_ura": subject,
        "session_id": session_id,
    }

subject = "easynet:///r/acme/resource/device.dev/window.recovery"

daemon = common("daemon_restart_active_session", subject, "sess-daemon-restart")
daemon["events"] = [
    event("PROCESS_STOPPED_UNCLEAN", subject, daemon["session_id"], 1000),
    event("DAEMON_RESTARTED", subject, daemon["session_id"], 2100),
    event("SESSION_REHYDRATED", subject, daemon["session_id"], 2600),
]
daemon["before_restart"] = {
    "session_id": daemon["session_id"],
    "selected_resource_ura": subject,
    "descriptor_version": "1.0.0",
    "target_binding_epoch": 7,
    "transport_epoch": 3,
}
daemon["after_restart"] = {
    "session_id": daemon["session_id"],
    "selected_resource_ura": subject,
    "descriptor_version": "1.0.0",
    "target_binding_epoch": 7,
    "transport_epoch": 3,
    "session_state": "active",
    "show_session_public": True,
    "show_session_observed_at_ms": 1787333003000,
    "watch_events_reattached": True,
    "watch_events_reattached_at_ms": 1787333003400,
    "media_reattached": True,
    "media_reattached_at_ms": 1787333003800,
    "frames_rendered_after_restart": 24,
    "first_frame_rendered_after_restart_at_ms": 1787333004300,
}
daemon["recovery"] = recovery(1, 2)
daemon["terminal_receipt"] = terminal(daemon["session_id"], "receipt-daemon")

plugin = common("plugin_worker_restart", subject, "sess-plugin-restart")
plugin["events"] = [
    event("PLUGIN_WORKER_CRASHED", subject, plugin["session_id"], 1000),
    event("PLUGIN_WORKER_RESTARTED", subject, plugin["session_id"], 1800),
    event("TARGET_MONITOR_RESTARTED", subject, plugin["session_id"], 2400),
]
plugin["same_public_session"] = True
plugin["media_source_epoch_before"] = 4
plugin["media_source_epoch_after"] = 5
plugin["frames_rendered_after_worker_restart"] = 31
plugin["first_frame_rendered_after_worker_restart_at_ms"] = 1787333003100
plugin["new_consent_required"] = False
plugin["recovery"] = recovery(2, 3)
plugin["terminal_receipt"] = terminal(plugin["session_id"], "receipt-plugin")

receipt = common("terminal_receipt_replay_after_crash", subject, "sess-receipt-replay")
receipt["events"] = [
    event("END_SESSION_ACCEPTED", subject, receipt["session_id"], 1000),
    event("PROCESS_STOPPED_UNCLEAN", subject, receipt["session_id"], 1300),
    event("TERMINAL_RECEIPT_REPLAYED", subject, receipt["session_id"], 2600),
]
receipt["terminal_receipt_before_crash"] = terminal(receipt["session_id"], "receipt-replayed")
receipt["terminal_receipt_after_restart"] = terminal(receipt["session_id"], "receipt-replayed")
receipt["repeat_end_session_idempotent"] = True
receipt["show_session_after_restart_state"] = "closed"
receipt["show_session_after_restart_observed_at_ms"] = 1787333003100
receipt["recovery"] = recovery(3, 4)

socket = common("stale_socket_restart_cleanup", subject, "sess-stale-socket")
socket["events"] = [
    event("STALE_CONTROL_SOCKET_DETECTED", subject, socket["session_id"], 1000),
    event("STALE_INVOCATION_SOCKET_DETECTED", subject, socket["session_id"], 1200),
    event("DAEMON_READY_AFTER_RESTART", subject, socket["session_id"], 2500),
]
socket["control_endpoint_ready"] = True
socket["invocation_endpoint_ready"] = True
socket["endpoint_ready_at_ms"] = 1787333002900
socket["stale_socket_cleanup_explicit"] = True
socket["manual_cleanup_required"] = False
socket["recovery"] = recovery(4, 5)
socket["terminal_receipt"] = terminal(socket["session_id"], "receipt-socket")

evidence = {
    "status": "passed",
    "proof_mode": "real_crash_restart_recovery_matrix",
    "component_mock": False,
    "real_backend_runtime": True,
    "product_complete_claim": False,
    "scenarios": [daemon, plugin, receipt, socket],
}
pathlib.Path(sys.argv[1]).write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
}

case "$MODE" in
  skip)
    write_report "skipped" "set EASYNET_REMOTEAPP_CRASH_RESTART_RECOVERY_E2E=1 or pass --run with real recovery evidence"
    echo "remoteapp-crash-restart-recovery-e2e skipped; report: $REPORT_JSON"
    ;;
  self-test)
    write_self_test_evidence
    validate_evidence
    echo "remoteapp-crash-restart-recovery-e2e self-test ok"
    ;;
  run)
    if [[ -n "$RUNNER_CMD" ]]; then
      EASYNET_REMOTEAPP_CRASH_RESTART_RECOVERY_EVIDENCE_JSON="$EVIDENCE_JSON" \
        bash -lc "$RUNNER_CMD" >"$RUNNER_STDOUT" 2>"$RUNNER_STDERR"
    elif [[ -n "$EVIDENCE_INPUT" ]]; then
      cp "$EVIDENCE_INPUT" "$EVIDENCE_JSON"
    else
      write_report "failed" "run mode requires --evidence-json or --runner-cmd"
      echo "remoteapp-crash-restart-recovery-e2e failed; report: $REPORT_JSON" >&2
      exit 64
    fi
    validate_evidence
    echo "remoteapp-crash-restart-recovery-e2e passed; report: $REPORT_JSON"
    ;;
  *)
    echo "unknown mode: $MODE" >&2
    exit 64
    ;;
esac
