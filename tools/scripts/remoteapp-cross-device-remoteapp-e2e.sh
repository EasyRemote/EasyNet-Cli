#!/usr/bin/env bash
# RemoteApp cross-device RemoteApp session artifact gate.
#
# This verifier validates externally collected cross-device RemoteApp evidence.
# It proves a stronger product path than the synthetic cross-device carrier:
# a Browser/network endpoint must run real RemoteApp display/window/application
# sessions on a distinct provider execution host, with remote target inventory,
# WebRTC/media rendering, input policy observation, and terminal receipts.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
PROVENANCE_HELPER="$SELF_DIR/remoteapp-evidence-provenance.py"
OUT_DIR="${EASYNET_REMOTEAPP_CROSS_DEVICE_REMOTEAPP_OUT_DIR:-$REPO_ROOT/target/e2e/remoteapp-cross-device-remoteapp/$(date -u +%Y%m%d-%H%M%S)-$$}"
EVIDENCE_INPUT="${EASYNET_REMOTEAPP_CROSS_DEVICE_REMOTEAPP_EVIDENCE_JSON:-}"
RUNNER_CMD="${EASYNET_REMOTEAPP_CROSS_DEVICE_REMOTEAPP_RUNNER_CMD:-}"
MODE=check

usage() {
  cat <<'USAGE'
Usage:
  tools/scripts/remoteapp-cross-device-remoteapp-e2e.sh --run --evidence-json evidence.json
  tools/scripts/remoteapp-cross-device-remoteapp-e2e.sh --self-test

Options:
  --run                 Validate externally collected live cross-device
                        RemoteApp evidence.
  --evidence-json PATH  Evidence JSON path. May also be supplied through
                        EASYNET_REMOTEAPP_CROSS_DEVICE_REMOTEAPP_EVIDENCE_JSON.
  --out-dir DIR         Report directory.
  --self-test           Validate this artifact contract with synthetic fixtures.
  -h, --help            Show this help.

Environment:
  EASYNET_REMOTEAPP_CROSS_DEVICE_REMOTEAPP_E2E=1
                        Allow --run validation.
  EASYNET_REMOTEAPP_CROSS_DEVICE_REMOTEAPP_RUNNER_CMD
                        Optional command that collects live evidence before
                        validation. The command must write the evidence path.

Evidence scope:
  The evidence JSON must prove a real RemoteApp cross-device product path:
  an admitted caller, Browser client endpoint, SystemAgent callee, provider
  execution Device, remote target inventory, display/window/application
  sessions, WebRTC media rendered on that client endpoint, input policy
  observation, governed ability calls, and terminal receipts.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --run) MODE=run; shift ;;
    --evidence-json) EVIDENCE_INPUT="${2:?missing value for --evidence-json}"; shift 2 ;;
    --out-dir) OUT_DIR="${2:?missing value for --out-dir}"; shift 2 ;;
    --self-test) MODE=self-test; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

write_report() {
  local status="$1"
  local reason="$2"
  mkdir -p "$OUT_DIR"
  python3 - "$OUT_DIR" "$status" "$reason" <<'PY'
import json
import pathlib
import sys

out_dir = pathlib.Path(sys.argv[1])
status = sys.argv[2]
reason = sys.argv[3]
report = {
    "script": "tools/scripts/remoteapp-cross-device-remoteapp-e2e.sh",
    "status": status,
    "reason": reason,
    "product_complete_claim": False,
    "coverage": {
        "remoteapp_cross_device_session": False,
        "display": False,
        "window": False,
        "application": False,
        "remote_media_rendered": False,
        "input_policy_checked": False,
        "remote_client_boundary_observed": False,
        "local_provider_boundary_only": True,
    },
}
(out_dir / "report.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
(out_dir / "report.md").write_text(
    "# RemoteApp Cross-Device RemoteApp E2E\n\n"
    f"- Status: `{status}`\n"
    f"- Reason: `{reason}`\n",
    encoding="utf-8",
)
PY
}

validate_evidence() {
  local evidence_path="$1"
  mkdir -p "$OUT_DIR"
  python3 "$PROVENANCE_HELPER" verify --mode "$MODE" --evidence "$evidence_path"
  python3 - "$evidence_path" "$OUT_DIR/report.json" "$OUT_DIR/report.md" <<'PY'
import json
import pathlib
import sys

evidence_path = pathlib.Path(sys.argv[1])
report_path = pathlib.Path(sys.argv[2])
md_path = pathlib.Path(sys.argv[3])

errors = []

def require(condition, message):
    if not condition:
        errors.append(message)

def is_ura(value):
    return isinstance(value, str) and value.startswith("easynet:///")

def has_ura_kind(value, kind):
    return is_ura(value) and f"/{kind}/" in value

def positive_int(value):
    try:
        return int(value) > 0
    except Exception:
        return False

def read_json(path):
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except Exception as exc:
        errors.append(f"invalid evidence JSON: {exc}")
        return {}

evidence = read_json(evidence_path)
if not isinstance(evidence, dict):
    errors.append("evidence root must be a JSON object")
    evidence = {}

required_targets = {"display", "window", "application"}
allowed_runner_kinds = {"two_device", "network_namespace", "deployment"}
terminal_reasons = {"caller_ended", "user_cancelled", "cross_device_remoteapp_e2e_cleanup"}

require(evidence.get("status") == "passed", "evidence.status must be passed")
require(evidence.get("proof_mode") == "real_remoteapp_cross_device_session",
        "proof_mode must be real_remoteapp_cross_device_session")
require(evidence.get("runner_kind") in allowed_runner_kinds,
        "runner_kind must be two_device, network_namespace, or deployment")
require(evidence.get("component_mock") is False, "component_mock must be false")
require(evidence.get("real_backend_runtime") is True, "real_backend_runtime must be true")
require(evidence.get("product_complete_claim") is False,
        "product_complete_claim must remain false")

topology = evidence.get("topology")
require(isinstance(topology, dict), "topology must be an object")
if not isinstance(topology, dict):
    topology = {}
require(topology.get("requires_remote_client_boundary") is True,
        "topology.requires_remote_client_boundary must be true")
require(topology.get("remote_client_boundary_observed") is True,
        "topology.remote_client_boundary_observed must be true")
require(topology.get("local_provider_boundary_only") is False,
        "topology.local_provider_boundary_only must be false")

observed_remote_endpoints = topology.get("observed_remote_endpoints")
require(isinstance(observed_remote_endpoints, list) and observed_remote_endpoints,
        "topology.observed_remote_endpoints must not be empty")
if not isinstance(observed_remote_endpoints, list):
    observed_remote_endpoints = []

remote_endpoint_bindings = []
for index, binding in enumerate(observed_remote_endpoints):
    if not isinstance(binding, dict):
        errors.append(f"topology.observed_remote_endpoints[{index}] must be an object")
        continue
    caller_ura = binding.get("caller_ura")
    callee_ura = binding.get("callee_ura")
    provider_device_ura = binding.get("provider_device_ura")
    client_endpoint_id = binding.get("client_endpoint_id")
    require(any(has_ura_kind(caller_ura, kind) for kind in ("user", "agent", "authority")),
            f"topology.observed_remote_endpoints[{index}].caller_ura must identify an admitted principal")
    require(has_ura_kind(callee_ura, "agent"),
            f"topology.observed_remote_endpoints[{index}].callee_ura must identify the SystemAgent")
    require(has_ura_kind(provider_device_ura, "device"),
            f"topology.observed_remote_endpoints[{index}].provider_device_ura must identify the execution host")
    require(isinstance(client_endpoint_id, str) and bool(client_endpoint_id),
            f"topology.observed_remote_endpoints[{index}].client_endpoint_id must identify the Browser peer")
    require(binding.get("remote_execution_boundary") is True,
            f"topology.observed_remote_endpoints[{index}].remote_execution_boundary must be true")
    if (
        any(has_ura_kind(caller_ura, kind) for kind in ("user", "agent", "authority"))
        and has_ura_kind(callee_ura, "agent")
        and has_ura_kind(provider_device_ura, "device")
        and isinstance(client_endpoint_id, str)
        and bool(client_endpoint_id)
        and binding.get("remote_execution_boundary") is True
    ):
        remote_endpoint_bindings.append((caller_ura, callee_ura, provider_device_ura, client_endpoint_id))
require(bool(remote_endpoint_bindings), "at least one remote Browser/provider endpoint binding must be observed")

scenarios = evidence.get("scenarios")
require(isinstance(scenarios, list) and scenarios,
        "scenarios must be a non-empty list")
if not isinstance(scenarios, list):
    scenarios = []

seen_targets = set()
scenario_reports = []
for index, scenario in enumerate(scenarios):
    if not isinstance(scenario, dict):
        errors.append(f"scenarios[{index}] must be an object")
        continue
    target_kind = scenario.get("target_kind")
    name = scenario.get("name") or f"scenario[{index}]"
    prefix = f"{name}/{target_kind}"
    require(target_kind in required_targets,
            f"{prefix}: target_kind must be one of {sorted(required_targets)}")
    if target_kind in required_targets:
        seen_targets.add(target_kind)
    require(scenario.get("status") == "passed", f"{prefix}: status must be passed")

    caller_ura = scenario.get("caller_ura")
    callee_ura = scenario.get("callee_ura")
    provider_device_ura = scenario.get("provider_device_ura")
    client_endpoint_id = scenario.get("client_endpoint_id")
    selected_resource_ura = scenario.get("selected_resource_ura")
    session_id = scenario.get("session_id")
    require(any(has_ura_kind(caller_ura, kind) for kind in ("user", "agent", "authority")),
            f"{prefix}: caller_ura must identify an admitted principal")
    require(has_ura_kind(callee_ura, "agent"),
            f"{prefix}: callee_ura must identify the Remote Desktop SystemAgent")
    require(has_ura_kind(provider_device_ura, "device"),
            f"{prefix}: provider_device_ura must identify the execution host")
    require(isinstance(client_endpoint_id, str) and bool(client_endpoint_id),
            f"{prefix}: client_endpoint_id must identify the Browser peer")
    require(scenario.get("remote_execution_boundary") is True,
            f"{prefix}: remote_execution_boundary must be true")
    require(is_ura(selected_resource_ura), f"{prefix}: selected_resource_ura must be canonical")
    require(isinstance(session_id, str) and session_id, f"{prefix}: session_id must be recorded")
    require(scenario.get("remote_target_inventory_seen") is True,
            f"{prefix}: remote_target_inventory_seen must be true")

    abilities = scenario.get("abilities")
    require(isinstance(abilities, list) and abilities, f"{prefix}: abilities must be non-empty")
    ability_by_name = {}
    if isinstance(abilities, list):
        for ability in abilities:
            if isinstance(ability, dict) and isinstance(ability.get("name"), str):
                ability_by_name[ability["name"]] = ability
    for ability_name in (
        "remote_desktop.create_session",
        "remote_desktop.set_description",
        "remote_desktop.watch_events",
        "remote_desktop.report_client_state",
        "remote_desktop.end_session",
    ):
        ability = ability_by_name.get(ability_name)
        require(isinstance(ability, dict), f"{prefix}: missing ability {ability_name}")
        if isinstance(ability, dict):
            require(ability.get("caller_ura") == caller_ura,
                    f"{prefix}: {ability_name} must bind caller_ura")
            require(ability.get("callee_ura") == callee_ura,
                    f"{prefix}: {ability_name} must bind callee_ura")
            require(ability.get("provider_device_ura") == provider_device_ura,
                    f"{prefix}: {ability_name} must bind provider_device_ura")
            require(ability.get("subject_ura") == selected_resource_ura,
                    f"{prefix}: {ability_name} must bind selected Resource URA")
            if ability_name != "remote_desktop.create_session":
                require(ability.get("session_id") == session_id,
                        f"{prefix}: {ability_name} must bind session_id")
    require("remote_desktop.attach" not in ability_by_name,
            f"{prefix}: diagnostic remote_desktop.attach cannot prove the production WebRTC path")

    capture = scenario.get("capture")
    require(isinstance(capture, dict), f"{prefix}: capture evidence must be an object")
    if not isinstance(capture, dict):
        capture = {}
    require(capture.get("provider_device_ura") == provider_device_ura,
            f"{prefix}: capture provider_device_ura must bind provider device")
    require(capture.get("selected_resource_ura") == selected_resource_ura,
            f"{prefix}: capture selected_resource_ura must bind selected Resource URA")
    require(capture.get("target_kind") == target_kind,
            f"{prefix}: capture target_kind must match scenario")
    require(capture.get("remote_target_inventory_seen") is True,
            f"{prefix}: capture remote_target_inventory_seen must be true")
    require(positive_int(capture.get("frames_captured")),
            f"{prefix}: capture.frames_captured must be positive")
    require(capture.get("counter_source") in {
                "provider_media_stats.frames_encoded",
                "provider_capture_stats.frames_captured",
            },
            f"{prefix}: capture.counter_source must identify a provider-side frame counter")

    media = scenario.get("media")
    require(isinstance(media, dict), f"{prefix}: media evidence must be an object")
    if not isinstance(media, dict):
        media = {}
    require(media.get("provider_device_ura") == provider_device_ura,
            f"{prefix}: media provider_device_ura must bind provider device")
    require(media.get("selected_resource_ura") == selected_resource_ura,
            f"{prefix}: media selected_resource_ura must bind selected Resource URA")
    require(media.get("session_id") == session_id,
            f"{prefix}: media session_id must bind session_id")
    require(media.get("transport") in {"webrtc", "easynet_relay_webrtc"},
            f"{prefix}: media transport must be WebRTC")
    require(media.get("production_media_pipeline") is True,
            f"{prefix}: media.production_media_pipeline must be true")
    require(media.get("peer_connection_state") == "connected",
            f"{prefix}: media.peer_connection_state must be connected")
    require(media.get("ice_connection_state") in {"connected", "completed"},
            f"{prefix}: media.ice_connection_state must be connected or completed")
    require(isinstance(media.get("selected_candidate_pair_id"), str)
            and bool(media.get("selected_candidate_pair_id")),
            f"{prefix}: media.selected_candidate_pair_id must be recorded")
    require(str(media.get("video_codec", "")).lower() == "h264",
            f"{prefix}: media.video_codec must be H264")
    require(positive_int(media.get("frames_rendered")),
            f"{prefix}: media.frames_rendered must be positive")
    require(media.get("rendered_after_connected") is True,
            f"{prefix}: media.rendered_after_connected must be true")
    require(media.get("rendered_on_client_endpoint") is True,
            f"{prefix}: media.rendered_on_client_endpoint must be true")
    require(media.get("client_endpoint_id") == client_endpoint_id,
            f"{prefix}: media.client_endpoint_id must bind the Browser peer")

    input_policy = scenario.get("input_policy")
    require(isinstance(input_policy, dict), f"{prefix}: input_policy evidence must be an object")
    if not isinstance(input_policy, dict):
        input_policy = {}
    require(input_policy.get("checked") is True,
            f"{prefix}: input_policy.checked must be true")
    require(input_policy.get("mode") in {"interactive", "view_only", "policy_blocked"},
            f"{prefix}: input_policy.mode must be interactive, view_only, or policy_blocked")
    require(input_policy.get("session_id") == session_id,
            f"{prefix}: input_policy session_id must bind session_id")

    terminal = scenario.get("terminal_receipt")
    require(isinstance(terminal, dict), f"{prefix}: terminal_receipt must be visible")
    if not isinstance(terminal, dict):
        terminal = {}
    require(terminal.get("terminal") is True,
            f"{prefix}: terminal_receipt.terminal must be true")
    require(terminal.get("receipt_type") == "remoteapp.session.terminal.v1",
            f"{prefix}: terminal_receipt.receipt_type must be remoteapp.session.terminal.v1")
    require(terminal.get("session_id") == session_id,
            f"{prefix}: terminal_receipt must bind session_id")
    require(terminal.get("subject_ura") == selected_resource_ura,
            f"{prefix}: terminal_receipt must bind selected Resource URA")
    require(terminal.get("reason_code") in terminal_reasons,
            f"{prefix}: terminal_receipt.reason_code must be a known cleanup/end reason")

    end_invocation_receipt = scenario.get("end_invocation_receipt")
    require(isinstance(end_invocation_receipt, dict),
            f"{prefix}: end_invocation_receipt must be visible")
    if not isinstance(end_invocation_receipt, dict):
        end_invocation_receipt = {}
    require(end_invocation_receipt.get("ability") == "remote_desktop.end_session",
            f"{prefix}: end_invocation_receipt must bind remote_desktop.end_session")
    require(isinstance(end_invocation_receipt.get("invocation_id"), str)
            and bool(end_invocation_receipt.get("invocation_id")),
            f"{prefix}: end_invocation_receipt.invocation_id must be recorded")
    require(is_ura(end_invocation_receipt.get("receipt_ura")),
            f"{prefix}: end_invocation_receipt.receipt_ura must be canonical")
    require(isinstance(end_invocation_receipt.get("receipt_hash"), str)
            and bool(end_invocation_receipt.get("receipt_hash")),
            f"{prefix}: end_invocation_receipt.receipt_hash must be recorded")
    require(end_invocation_receipt.get("verified") is True,
            f"{prefix}: end_invocation_receipt.verified must be true")

    abilities_bound = True
    for ability_name in (
        "remote_desktop.create_session",
        "remote_desktop.set_description",
        "remote_desktop.watch_events",
        "remote_desktop.report_client_state",
        "remote_desktop.end_session",
    ):
        ability = ability_by_name.get(ability_name)
        if not isinstance(ability, dict):
            abilities_bound = False
            continue
        if ability.get("caller_ura") != caller_ura:
            abilities_bound = False
        if ability.get("callee_ura") != callee_ura:
            abilities_bound = False
        if ability.get("provider_device_ura") != provider_device_ura:
            abilities_bound = False
        if ability.get("subject_ura") != selected_resource_ura:
            abilities_bound = False
        if ability_name != "remote_desktop.create_session" and ability.get("session_id") != session_id:
            abilities_bound = False

    scenario_reports.append({
        "name": name,
        "target_kind": target_kind,
        "caller_ura": caller_ura,
        "callee_ura": callee_ura,
        "provider_device_ura": provider_device_ura,
        "client_endpoint_id": client_endpoint_id,
        "remote_execution_boundary": scenario.get("remote_execution_boundary") is True,
        "selected_resource_ura": selected_resource_ura,
        "session_id": session_id,
        "frames_captured": capture.get("frames_captured"),
        "frames_rendered": media.get("frames_rendered"),
        "input_policy_mode": input_policy.get("mode"),
        "remoteapp_summary": {
            "caller_ura": caller_ura,
            "callee_ura": callee_ura,
            "provider_device_ura": provider_device_ura,
            "client_endpoint_id": client_endpoint_id,
            "selected_resource_ura": selected_resource_ura,
            "session_id": session_id,
            "remote_execution_boundary": scenario.get("remote_execution_boundary") is True,
            "remote_target_inventory_seen": scenario.get("remote_target_inventory_seen") is True,
            "abilities_bound": abilities_bound,
            "production_signaling_bound": abilities_bound,
            "diagnostic_attach_absent": "remote_desktop.attach" not in ability_by_name,
            "capture_provider_bound": capture.get("provider_device_ura") == provider_device_ura,
            "capture_resource_bound": capture.get("selected_resource_ura") == selected_resource_ura,
            "capture_target_kind_bound": capture.get("target_kind") == target_kind,
            "capture_remote_target_inventory_seen": capture.get("remote_target_inventory_seen") is True,
            "capture_frames_captured": capture.get("frames_captured"),
            "capture_counter_source": capture.get("counter_source"),
            "media_provider_bound": media.get("provider_device_ura") == provider_device_ura,
            "media_resource_bound": media.get("selected_resource_ura") == selected_resource_ura,
            "media_session_bound": media.get("session_id") == session_id,
            "media_transport": media.get("transport"),
            "production_media_pipeline": media.get("production_media_pipeline") is True,
            "peer_connection_state": media.get("peer_connection_state"),
            "ice_connection_state": media.get("ice_connection_state"),
            "selected_candidate_pair_id": media.get("selected_candidate_pair_id"),
            "video_codec": media.get("video_codec"),
            "media_frames_rendered": media.get("frames_rendered"),
            "rendered_after_connected": media.get("rendered_after_connected") is True,
            "rendered_on_client_endpoint": media.get("rendered_on_client_endpoint") is True,
            "client_endpoint_bound": media.get("client_endpoint_id") == client_endpoint_id,
            "input_policy_checked": input_policy.get("checked") is True,
            "input_policy_mode": input_policy.get("mode"),
            "input_policy_session_bound": input_policy.get("session_id") == session_id,
            "terminal_receipt_visible": terminal.get("terminal") is True,
            "terminal_receipt_session_bound": terminal.get("session_id") == session_id,
            "terminal_receipt_subject_bound": terminal.get("subject_ura") == selected_resource_ura,
            "end_invocation_receipt_verified": end_invocation_receipt.get("verified") is True,
            "end_invocation_receipt_ura": end_invocation_receipt.get("receipt_ura"),
            "end_invocation_receipt_hash": end_invocation_receipt.get("receipt_hash"),
            "terminal_reason": terminal.get("reason_code"),
        },
    })

missing_targets = sorted(required_targets - seen_targets)
require(not missing_targets, "missing target scenarios: " + ", ".join(missing_targets))

coverage = {
    "remoteapp_cross_device_session": not errors,
    "display": "display" in seen_targets,
    "window": "window" in seen_targets,
    "application": "application" in seen_targets,
    "remote_media_rendered": all(positive_int(item.get("frames_rendered")) for item in scenario_reports) and bool(scenario_reports),
    "input_policy_checked": len(scenario_reports) == len(scenarios),
    "remote_client_boundary_observed": bool(remote_endpoint_bindings),
    "local_provider_boundary_only": not bool(remote_endpoint_bindings),
}

report = {
    "script": "tools/scripts/remoteapp-cross-device-remoteapp-e2e.sh",
    "status": "failed" if errors else "passed",
    "errors": errors,
    "coverage": coverage,
    "scenario_count": len(scenario_reports),
    "scenarios": scenario_reports,
    "evidence_json": str(evidence_path),
    "product_complete_claim": False,
}
unique_endpoint_bindings = []
for binding in remote_endpoint_bindings:
    if binding not in unique_endpoint_bindings:
        unique_endpoint_bindings.append(binding)
report["topology"] = {
    "requires_remote_client_boundary": True,
    "observed_remote_endpoints": [
        {
            "caller_ura": caller_ura,
            "callee_ura": callee_ura,
            "provider_device_ura": provider_device_ura,
            "client_endpoint_id": client_endpoint_id,
            "remote_execution_boundary": True,
        }
        for caller_ura, callee_ura, provider_device_ura, client_endpoint_id in unique_endpoint_bindings
    ],
    "remote_client_boundary_observed": bool(remote_endpoint_bindings),
    "local_provider_boundary_only": not bool(remote_endpoint_bindings),
}
report_path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
with open(md_path, "w", encoding="utf-8") as f:
    f.write("# RemoteApp Cross-Device RemoteApp E2E\n\n")
    f.write(f"- Status: `{report['status']}`\n")
    f.write(f"- Evidence: `{evidence_path}`\n")
    for target in sorted(required_targets):
        f.write(f"- {target}: `{str(coverage[target]).lower()}`\n")
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
    --evidence "$evidence_path" --report "$OUT_DIR/report.json"
}

write_self_test_evidence() {
  local path="$1"
  python3 - "$path" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
caller = "easynet:///r/localhost/user/caller-user"
callee = "easynet:///r/localhost/agent/device.provider-device.remote-desktop"
provider = "easynet:///r/localhost/device/provider-device"
client_endpoint = "browser-peer-contract"
scenarios = []
for target_kind in ("display", "window", "application"):
    session_id = f"rd-cross-device-{target_kind}"
    selected_resource_ura = f"easynet:///r/localhost/resource/device.provider/{target_kind}.selected"
    scenarios.append({
        "name": f"{target_kind}-remoteapp",
        "status": "passed",
        "target_kind": target_kind,
        "caller_ura": caller,
        "callee_ura": callee,
        "provider_device_ura": provider,
        "client_endpoint_id": client_endpoint,
        "remote_execution_boundary": True,
        "selected_resource_ura": selected_resource_ura,
        "session_id": session_id,
        "remote_target_inventory_seen": True,
        "abilities": [
            {"name": "remote_desktop.create_session", "caller_ura": caller, "callee_ura": callee, "provider_device_ura": provider, "subject_ura": selected_resource_ura},
            {"name": "remote_desktop.set_description", "caller_ura": caller, "callee_ura": callee, "provider_device_ura": provider, "subject_ura": selected_resource_ura, "session_id": session_id},
            {"name": "remote_desktop.watch_events", "caller_ura": caller, "callee_ura": callee, "provider_device_ura": provider, "subject_ura": selected_resource_ura, "session_id": session_id},
            {"name": "remote_desktop.report_client_state", "caller_ura": caller, "callee_ura": callee, "provider_device_ura": provider, "subject_ura": selected_resource_ura, "session_id": session_id},
            {"name": "remote_desktop.end_session", "caller_ura": caller, "callee_ura": callee, "provider_device_ura": provider, "subject_ura": selected_resource_ura, "session_id": session_id},
        ],
        "capture": {
            "provider_device_ura": provider,
            "selected_resource_ura": selected_resource_ura,
            "target_kind": target_kind,
            "remote_target_inventory_seen": True,
            "frames_captured": 12,
            "counter_source": "provider_media_stats.frames_encoded",
        },
        "media": {
            "provider_device_ura": provider,
            "selected_resource_ura": selected_resource_ura,
            "session_id": session_id,
            "transport": "webrtc",
            "production_media_pipeline": True,
            "peer_connection_state": "connected",
            "ice_connection_state": "connected",
            "selected_candidate_pair_id": f"pair-{target_kind}",
            "video_codec": "H264",
            "frames_rendered": 10,
            "rendered_after_connected": True,
            "rendered_on_client_endpoint": True,
            "client_endpoint_id": client_endpoint,
        },
        "input_policy": {
            "checked": True,
            "mode": "view_only",
            "session_id": session_id,
        },
        "terminal_receipt": {
            "receipt_type": "remoteapp.session.terminal.v1",
            "terminal": True,
            "session_id": session_id,
            "subject_ura": selected_resource_ura,
            "reason_code": "cross_device_remoteapp_e2e_cleanup",
        },
        "end_invocation_receipt": {
            "ability": "remote_desktop.end_session",
            "invocation_id": f"inv-{session_id}",
            "receipt_ura": f"easynet:///r/localhost/receipt/{session_id}",
            "receipt_hash": f"hash-{session_id}",
            "verified": True,
        },
    })
evidence = {
    "status": "passed",
    "evidence_origin": "contract_self_test",
    "proof_mode": "real_remoteapp_cross_device_session",
    "runner_kind": "two_device",
    "component_mock": False,
    "real_backend_runtime": True,
    "product_complete_claim": False,
    "topology": {
        "requires_remote_client_boundary": True,
        "observed_remote_endpoints": [
            {
                "caller_ura": caller,
                "callee_ura": callee,
                "provider_device_ura": provider,
                "client_endpoint_id": client_endpoint,
                "remote_execution_boundary": True,
            }
        ],
        "remote_client_boundary_observed": True,
        "local_provider_boundary_only": False,
    },
    "scenarios": scenarios,
}
path.parent.mkdir(parents=True, exist_ok=True)
path.write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
}

run_self_test() {
  self_test_tmp="$(mktemp -d)"
  trap 'if [[ -n "${self_test_tmp:-}" ]]; then rm -rf "$self_test_tmp"; fi' EXIT
  local tmp="$self_test_tmp"
  write_self_test_evidence "$tmp/evidence.json"
  python3 - "$tmp/evidence.json" "$tmp/live-evidence.json" <<'PY'
import json
import pathlib
import sys

source = pathlib.Path(sys.argv[1])
target = pathlib.Path(sys.argv[2])
evidence = json.loads(source.read_text(encoding="utf-8"))
evidence["evidence_origin"] = "live_runner"
target.write_text(json.dumps(evidence) + "\n", encoding="utf-8")
PY
  EASYNET_REMOTEAPP_CROSS_DEVICE_REMOTEAPP_E2E=1 "$0" --run --evidence-json "$tmp/live-evidence.json" --out-dir "$tmp/pass" >/dev/null

  python3 - "$tmp/live-evidence.json" "$tmp/missing-application.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["scenarios"] = [
    scenario for scenario in evidence["scenarios"]
    if scenario["target_kind"] != "application"
]
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
  if EASYNET_REMOTEAPP_CROSS_DEVICE_REMOTEAPP_E2E=1 "$0" --run --evidence-json "$tmp/missing-application.json" --out-dir "$tmp/missing-application" >/dev/null 2>&1; then
    echo "self-test accepted evidence without application target scenario" >&2
    exit 1
  fi

  python3 - "$tmp/live-evidence.json" "$tmp/local-provider.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["topology"]["observed_remote_endpoints"][0]["remote_execution_boundary"] = False
evidence["topology"]["remote_client_boundary_observed"] = False
evidence["topology"]["local_provider_boundary_only"] = True
for scenario in evidence["scenarios"]:
    scenario["remote_execution_boundary"] = False
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
  if EASYNET_REMOTEAPP_CROSS_DEVICE_REMOTEAPP_E2E=1 "$0" --run --evidence-json "$tmp/local-provider.json" --out-dir "$tmp/local-provider" >/dev/null 2>&1; then
    echo "self-test accepted local-provider-only RemoteApp evidence" >&2
    exit 1
  fi

  python3 - "$tmp/live-evidence.json" "$tmp/no-media.json" <<'PY'
import json
import sys

evidence = json.load(open(sys.argv[1], encoding="utf-8"))
evidence["scenarios"][0]["media"]["frames_rendered"] = 0
json.dump(evidence, open(sys.argv[2], "w", encoding="utf-8"), indent=2)
PY
  if EASYNET_REMOTEAPP_CROSS_DEVICE_REMOTEAPP_E2E=1 "$0" --run --evidence-json "$tmp/no-media.json" --out-dir "$tmp/no-media" >/dev/null 2>&1; then
    echo "self-test accepted RemoteApp evidence without rendered media" >&2
    exit 1
  fi

  mkdir -p "$OUT_DIR"
  cp "$tmp/evidence.json" "$OUT_DIR/evidence.json"
  echo "remoteapp-cross-device-remoteapp-e2e self-test ok"
}

case "$MODE" in
  check)
    write_report "skipped" "set EASYNET_REMOTEAPP_CROSS_DEVICE_REMOTEAPP_E2E=1 and pass --run with live evidence"
    echo "[remoteapp-cross-device-remoteapp-e2e] SKIP: $OUT_DIR/report.md"
    ;;
  run)
    if [[ "${EASYNET_REMOTEAPP_CROSS_DEVICE_REMOTEAPP_E2E:-0}" != "1" ]]; then
      write_report "skipped" "set EASYNET_REMOTEAPP_CROSS_DEVICE_REMOTEAPP_E2E=1 to validate live evidence"
      echo "[remoteapp-cross-device-remoteapp-e2e] SKIP: $OUT_DIR/report.md"
      exit 0
    fi
    if [[ -n "$RUNNER_CMD" ]]; then
      export EASYNET_REMOTEAPP_CROSS_DEVICE_REMOTEAPP_EVIDENCE_JSON="$EVIDENCE_INPUT"
      bash -lc "$RUNNER_CMD"
    fi
    if [[ -z "$EVIDENCE_INPUT" ]]; then
      echo "--evidence-json or EASYNET_REMOTEAPP_CROSS_DEVICE_REMOTEAPP_EVIDENCE_JSON is required for --run" >&2
      exit 64
    fi
    validate_evidence "$EVIDENCE_INPUT"
    echo "[remoteapp-cross-device-remoteapp-e2e] PASS: $OUT_DIR/report.md"
    ;;
  self-test)
    bash -n "$0"
    grep -q "real_remoteapp_cross_device_session" "$0"
    grep -q "remote_desktop.create_session" "$0"
    grep -q "remote_desktop.set_description" "$0"
    grep -q "diagnostic remote_desktop.attach cannot prove the production WebRTC path" "$0"
    grep -q "selected_candidate_pair_id" "$0"
    grep -q "remote_target_inventory_seen" "$0"
    grep -q "remoteapp_summary" "$0"
    grep -q "rendered_on_client_endpoint" "$0"
    grep -q "client_endpoint_id" "$0"
    grep -q "input_policy.checked must be true" "$0"
    grep -q "missing target scenarios" "$0"
    grep -q "self-test accepted local-provider-only RemoteApp evidence" "$0"
    run_self_test
    ;;
esac
