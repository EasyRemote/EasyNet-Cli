#!/usr/bin/env bash
# RemoteApp product-completion evidence gate.
#
# This gate is the only place that may aggregate individual RemoteApp E2E
# evidence into a product-complete claim. It does not implement capture, input,
# media, network, frontend, or cross-device behavior; those semantics remain in
# their dedicated verifiers. This script fails closed when any required report is
# missing, failed, local-provider-only, or still a source/self-test non-claim.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
OUT_DIR="${EASYNET_REMOTEAPP_PRODUCT_COMPLETION_OUT_DIR:-$REPO_ROOT/target/e2e/remoteapp-product-completion/$(date -u +%Y%m%d-%H%M%S)-$$}"
MODE=check

usage() {
  cat <<'USAGE'
Usage:
  tools/scripts/remoteapp-product-completion-e2e.sh --check
  tools/scripts/remoteapp-product-completion-e2e.sh --self-test

Options:
  --check       Aggregate required RemoteApp product evidence reports.
  --self-test   Validate this gate with synthetic pass/fail reports.
  --out-dir DIR Report directory.
  -h, --help    Show this help.

Required report environment:
  EASYNET_REMOTEAPP_PRODUCT_COMPLETION_FRONTEND_PRODUCT_FLOW_REPORT_JSON
  EASYNET_REMOTEAPP_PRODUCT_COMPLETION_BROWSER_LIFECYCLE_REPORT_JSON
  EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_SMOKE_REPORT_JSON
  EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_REMOTEAPP_REPORT_JSON
  EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_PLATFORM_CAPTURE_REPORT_JSON
  EASYNET_REMOTEAPP_PRODUCT_COMPLETION_INPUT_INJECTION_REPORT_JSON
  EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MEDIA_ADAPTATION_REPORT_JSON
  EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MULTI_WINDOW_TRACKING_REPORT_JSON
  EASYNET_REMOTEAPP_PRODUCT_COMPLETION_NETWORK_FALLBACK_REPORT_JSON
  EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_TIMEOUT_WINDOW_REPORT_JSON
  EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_TIMEOUT_APPLICATION_REPORT_JSON
  EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_CANCEL_WINDOW_REPORT_JSON
  EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_CANCEL_APPLICATION_REPORT_JSON
  EASYNET_REMOTEAPP_PRODUCT_COMPLETION_PERMISSION_REVOKE_WINDOW_REPORT_JSON
  EASYNET_REMOTEAPP_PRODUCT_COMPLETION_PERMISSION_REVOKE_APPLICATION_REPORT_JSON
  EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_RESUME_WINDOW_REPORT_JSON
  EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_RESUME_APPLICATION_REPORT_JSON
  EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CRASH_RESTART_RECOVERY_REPORT_JSON

Evidence scope:
  This gate is a product-completion aggregator. It can pass only when every
  required live report passes. It is not a replacement for any per-domain
  verifier and it rejects local-provider-only cross-device evidence.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --check) MODE=check; shift ;;
    --self-test) MODE=self-test; shift ;;
    --out-dir) OUT_DIR="${2:?missing value for --out-dir}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

write_completion_report() {
  local mode="$1"
  python3 - "$OUT_DIR" "$mode" <<'PY'
import json
import os
import pathlib
import sys

out_dir = pathlib.Path(sys.argv[1])
mode = sys.argv[2]
out_dir.mkdir(parents=True, exist_ok=True)

lifecycle_targets = ("window", "application")

def lifecycle_required(item_prefix, env_prefix, expected_script):
    return [
        {
            "id": f"{item_prefix}_{target_kind}",
            "env": f"{env_prefix}_{target_kind.upper()}_REPORT_JSON",
            "expected_script": expected_script,
            "expected_target_kind": target_kind,
            "coverage_keys": [],
            "requires_evidence_json": True,
        }
        for target_kind in lifecycle_targets
    ]

required = [
    {
        "id": "frontend_product_flow",
        "env": "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_FRONTEND_PRODUCT_FLOW_REPORT_JSON",
        "expected_script": "tools/scripts/frontend-remoteapp-product-flow-e2e.sh",
        "expected_target_kind": "both",
        "coverage_keys": [],
        "required_steps": [
            "hub-api-readiness-preflight",
            "product-runtime-readiness-preflight",
            "frontend-typecheck",
            "frontend-remoteapp-ui-flow",
            "frontend-browser-lifecycle",
            "cross-device-product-smoke",
            "host-permission-subject",
            "host-target-picker-freshness",
            "host-decoded-frame-window",
            "host-decoded-frame-application",
            "host-view-only-input-window",
            "host-view-only-input-application",
        ],
        "evidence_contract_contains": [
            "Browser/Tauri RemoteApp lifecycle evidence",
            "cross-device product smoke with distinct device URAs",
        ],
        "product_flow_step_artifacts": [
            {"name": "hub-api-readiness-preflight"},
            {"name": "product-runtime-readiness-preflight"},
            {"name": "frontend-typecheck"},
            {"name": "frontend-remoteapp-ui-flow"},
            {
                "name": "frontend-browser-lifecycle",
                "report_json": "report.json",
                "expected_script": "tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh",
                "requires_evidence_json": True,
            },
            {
                "name": "cross-device-product-smoke",
                "report_json": "evidence-report.json",
                "expected_script": "tools/scripts/remoteapp-cross-device-product-smoke.sh",
                "cross_device": True,
            },
            {
                "name": "host-permission-subject",
                "report_json": "report.json",
                "expected_script": "tools/scripts/host-remoteapp-permission-subject-e2e.sh",
                "requires_evidence_json": True,
            },
            {
                "name": "host-target-picker-freshness",
                "report_json": "report.json",
                "expected_script": "tools/scripts/host-remoteapp-target-picker-freshness-e2e.sh",
                "requires_evidence_json": True,
            },
            {
                "name": "host-decoded-frame-window",
                "report_json": "report.json",
                "expected_script": "tools/scripts/host-remoteapp-decoded-frame-e2e.sh",
                "expected_target_kind": "window",
                "requires_evidence_json": True,
            },
            {
                "name": "host-decoded-frame-application",
                "report_json": "report.json",
                "expected_script": "tools/scripts/host-remoteapp-decoded-frame-e2e.sh",
                "expected_target_kind": "application",
                "requires_evidence_json": True,
            },
            {
                "name": "host-view-only-input-window",
                "report_json": "report.json",
                "expected_script": "tools/scripts/host-remoteapp-view-only-input-safety-e2e.sh",
                "expected_target_kind": "window",
                "requires_evidence_json": True,
            },
            {
                "name": "host-view-only-input-application",
                "report_json": "report.json",
                "expected_script": "tools/scripts/host-remoteapp-view-only-input-safety-e2e.sh",
                "expected_target_kind": "application",
                "requires_evidence_json": True,
            },
        ],
    },
    {
        "id": "browser_lifecycle",
        "env": "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_BROWSER_LIFECYCLE_REPORT_JSON",
        "expected_script": "tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh",
        "coverage_keys": [],
        "requires_evidence_json": True,
    },
    {
        "id": "cross_device_smoke",
        "env": "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_SMOKE_REPORT_JSON",
        "expected_script": "tools/scripts/remoteapp-cross-device-product-smoke.sh",
        "coverage_keys": [
            "cross_device_hub_routing",
            "synthetic_stream_bidi_carrier",
            "distinct_device_uras_observed",
        ],
        "cross_device": True,
        "requires_observed_device_pairs": True,
    },
    {
        "id": "cross_device_remoteapp",
        "env": "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_REMOTEAPP_REPORT_JSON",
        "expected_script": "tools/scripts/remoteapp-cross-device-remoteapp-e2e.sh",
        "coverage_keys": [
            "remoteapp_cross_device_session",
            "display",
            "window",
            "application",
            "remote_media_rendered",
            "input_policy_checked",
            "distinct_device_uras_observed",
        ],
        "cross_device": True,
        "requires_observed_device_pairs": True,
        "requires_evidence_json": True,
        "requires_cross_device_remoteapp_scenarios": True,
    },
    {
        "id": "cross_platform_capture",
        "env": "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_PLATFORM_CAPTURE_REPORT_JSON",
        "expected_script": "tools/scripts/remoteapp-cross-platform-capture-e2e.sh",
        "coverage_keys": ["macos", "windows", "linux"],
        "requires_evidence_json": True,
        "requires_platforms_passed": ["macos", "windows", "linux"],
        "requires_passed_targets": ["display", "window", "application"],
    },
    {
        "id": "input_injection",
        "env": "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_INPUT_INJECTION_REPORT_JSON",
        "expected_script": "tools/scripts/remoteapp-input-injection-e2e.sh",
        "coverage_keys": ["macos", "windows", "linux"],
        "requires_evidence_json": True,
        "requires_platforms_passed": ["macos", "windows", "linux"],
    },
    {
        "id": "media_adaptation",
        "env": "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MEDIA_ADAPTATION_REPORT_JSON",
        "expected_script": "tools/scripts/remoteapp-media-adaptation-e2e.sh",
        "coverage_keys": ["baseline", "degraded_network", "backpressure"],
        "requires_evidence_json": True,
    },
    {
        "id": "multi_window_tracking",
        "env": "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MULTI_WINDOW_TRACKING_REPORT_JSON",
        "expected_script": "tools/scripts/remoteapp-multi-window-tracking-e2e.sh",
        "coverage_keys": [
            "independent_window_streams",
            "geometry_churn",
            "application_window_set_churn",
            "target_loss_rebind",
            "multi_display_application",
        ],
        "requires_evidence_json": True,
    },
    {
        "id": "network_fallback",
        "env": "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_NETWORK_FALLBACK_REPORT_JSON",
        "expected_script": "tools/scripts/remoteapp-network-fallback-e2e.sh",
        "coverage_keys": ["direct", "stun_srflx", "turn_relay", "easynet_relay"],
        "requires_evidence_json": True,
        "requires_network_route_scenarios": True,
    },
    *lifecycle_required(
        "session_timeout",
        "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_TIMEOUT",
        "tools/scripts/host-remoteapp-session-timeout-e2e.sh",
    ),
    *lifecycle_required(
        "session_cancel",
        "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_CANCEL",
        "tools/scripts/host-remoteapp-session-cancel-e2e.sh",
    ),
    *lifecycle_required(
        "permission_revoke",
        "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_PERMISSION_REVOKE",
        "tools/scripts/host-remoteapp-permission-revoke-e2e.sh",
    ),
    *lifecycle_required(
        "session_resume",
        "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_RESUME",
        "tools/scripts/host-remoteapp-session-resume-e2e.sh",
    ),
    {
        "id": "crash_restart_recovery",
        "env": "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CRASH_RESTART_RECOVERY_REPORT_JSON",
        "expected_script": "tools/scripts/remoteapp-crash-restart-recovery-e2e.sh",
        "coverage_keys": [
            "daemon_restart_active_session",
            "plugin_worker_restart",
            "terminal_receipt_replay_after_crash",
            "stale_socket_restart_cleanup",
        ],
        "requires_evidence_json": True,
    },
]

checks = []
errors = []

def add_error(item_id, message):
    errors.append(f"{item_id}: {message}")

def read_required_evidence_json(item_id, check, evidence_path, label):
    try:
        evidence = json.loads(evidence_path.read_text(encoding="utf-8"))
    except Exception as exc:
        message = f"{label} invalid evidence_json: {exc}"
        check["errors"].append(message)
        add_error(item_id, message)
        return None
    if not isinstance(evidence, dict):
        message = f"{label} evidence_json must contain a JSON object"
        check["errors"].append(message)
        add_error(item_id, message)
        return None
    if evidence.get("status") != "passed":
        message = f"{label} evidence_json status is {evidence.get('status')!r}, expected 'passed'"
        check["errors"].append(message)
        add_error(item_id, message)
    return evidence

def lower(value):
    return value.lower() if isinstance(value, str) else value

def positive_int(value):
    try:
        return int(value) > 0
    except Exception:
        return False

def validate_network_route_scenarios(item_id, check, report):
    required_routes = {
        "direct": {
            "selected_route_class": "direct",
            "required_candidate_types": {"host"},
            "forbidden_candidate_types": {"relay"},
            "required_allowed_classes": {"direct"},
            "required_blocked_classes": {"relay"},
        },
        "stun_srflx": {
            "selected_route_class": "stun_srflx",
            "required_candidate_types": {"srflx", "prflx"},
            "required_allowed_classes": {"stun_srflx"},
            "required_blocked_classes": {"direct"},
        },
        "turn_relay": {
            "selected_route_class": "relay",
            "required_candidate_types": {"relay"},
            "required_allowed_classes": {"relay"},
            "required_blocked_classes": {"direct", "stun_srflx"},
        },
        "easynet_relay": {
            "selected_route_class": "relay",
            "required_candidate_types": {"relay"},
            "required_allowed_classes": {"relay"},
            "required_blocked_classes": {"direct", "stun_srflx"},
        },
    }
    scenarios = report.get("scenarios")
    check["required_network_routes"] = sorted(required_routes)
    if not isinstance(scenarios, list) or not scenarios:
        message = "network fallback scenarios summary must be a non-empty list"
        check["errors"].append(message)
        add_error(item_id, message)
        return

    route_entries = {}
    for index, scenario in enumerate(scenarios):
        if not isinstance(scenario, dict):
            message = f"network fallback scenarios[{index}] must be an object"
            check["errors"].append(message)
            add_error(item_id, message)
            continue
        route_kind = scenario.get("route_kind")
        if route_kind not in required_routes:
            message = f"network fallback scenarios[{index}].route_kind is {route_kind!r}, expected one of {sorted(required_routes)}"
            check["errors"].append(message)
            add_error(item_id, message)
            continue
        if route_kind in route_entries:
            message = f"network fallback route {route_kind!r} appears more than once"
            check["errors"].append(message)
            add_error(item_id, message)
        route_entries[route_kind] = scenario

    observed_routes = sorted(route_entries)
    check["observed_network_routes"] = observed_routes
    missing_routes = sorted(set(required_routes) - set(route_entries))
    if missing_routes:
        message = "network fallback scenarios missing routes: " + ", ".join(missing_routes)
        check["errors"].append(message)
        add_error(item_id, message)

    for route_kind, spec in required_routes.items():
        scenario = route_entries.get(route_kind)
        if not isinstance(scenario, dict):
            continue
        prefix = f"network fallback route {route_kind}"
        selected_route_class = lower(scenario.get("selected_route_class"))
        if selected_route_class != spec["selected_route_class"]:
            message = f"{prefix}: selected_route_class is {selected_route_class!r}, expected {spec['selected_route_class']!r}"
            check["errors"].append(message)
            add_error(item_id, message)
        if scenario.get("ice_connection_state") not in {"connected", "completed"}:
            message = f"{prefix}: ice_connection_state must be connected or completed"
            check["errors"].append(message)
            add_error(item_id, message)
        if not isinstance(scenario.get("candidate_pair_id"), str) or not scenario.get("candidate_pair_id"):
            message = f"{prefix}: candidate_pair_id must be set"
            check["errors"].append(message)
            add_error(item_id, message)
        if not isinstance(scenario.get("session_id"), str) or not scenario.get("session_id"):
            message = f"{prefix}: session_id must be set"
            check["errors"].append(message)
            add_error(item_id, message)
        if not positive_int(scenario.get("frames_rendered")):
            message = f"{prefix}: frames_rendered must be positive"
            check["errors"].append(message)
            add_error(item_id, message)

        candidate_types = {
            lower(item)
            for item in scenario.get("candidate_types", [])
            if isinstance(item, str)
        }
        allowed_route_classes = {
            lower(item)
            for item in scenario.get("allowed_route_classes", [])
            if isinstance(item, str)
        }
        blocked_route_classes = {
            lower(item)
            for item in scenario.get("blocked_route_classes", [])
            if isinstance(item, str)
        }
        if not spec["required_allowed_classes"].issubset(allowed_route_classes):
            message = f"{prefix}: allowed_route_classes must include {sorted(spec['required_allowed_classes'])}"
            check["errors"].append(message)
            add_error(item_id, message)
        if not spec["required_blocked_classes"].issubset(blocked_route_classes):
            message = f"{prefix}: blocked_route_classes must include {sorted(spec['required_blocked_classes'])}"
            check["errors"].append(message)
            add_error(item_id, message)
        if selected_route_class not in allowed_route_classes:
            message = f"{prefix}: selected_route_class must be allowed by the network fixture"
            check["errors"].append(message)
            add_error(item_id, message)
        if selected_route_class in blocked_route_classes:
            message = f"{prefix}: selected_route_class must not be blocked by the network fixture"
            check["errors"].append(message)
            add_error(item_id, message)
        if not spec["required_candidate_types"].intersection(candidate_types):
            message = f"{prefix}: candidate_types must include one of {sorted(spec['required_candidate_types'])}"
            check["errors"].append(message)
            add_error(item_id, message)
        forbidden_candidate_types = spec.get("forbidden_candidate_types", set())
        if forbidden_candidate_types.intersection(candidate_types):
            message = f"{prefix}: candidate_types must not include {sorted(forbidden_candidate_types)}"
            check["errors"].append(message)
            add_error(item_id, message)

def validate_cross_device_remoteapp_scenarios(item_id, check, report):
    required_targets = {"display", "window", "application"}
    scenarios = report.get("scenarios")
    check["required_cross_device_remoteapp_targets"] = sorted(required_targets)
    if not isinstance(scenarios, list) or not scenarios:
        message = "cross-device RemoteApp scenarios summary must be a non-empty list"
        check["errors"].append(message)
        add_error(item_id, message)
        return

    seen_targets = set()
    for index, scenario in enumerate(scenarios):
        if not isinstance(scenario, dict):
            message = f"cross-device RemoteApp scenarios[{index}] must be an object"
            check["errors"].append(message)
            add_error(item_id, message)
            continue
        target_kind = scenario.get("target_kind")
        if target_kind not in required_targets:
            message = f"cross-device RemoteApp scenarios[{index}].target_kind is {target_kind!r}, expected one of {sorted(required_targets)}"
            check["errors"].append(message)
            add_error(item_id, message)
            continue
        if target_kind in seen_targets:
            message = f"cross-device RemoteApp target {target_kind!r} appears more than once"
            check["errors"].append(message)
            add_error(item_id, message)
        seen_targets.add(target_kind)
        prefix = f"cross-device RemoteApp target {target_kind}"

        caller_device_ura = scenario.get("caller_device_ura")
        provider_device_ura = scenario.get("provider_device_ura")
        selected_resource_ura = scenario.get("selected_resource_ura")
        session_id = scenario.get("session_id")
        if not isinstance(caller_device_ura, str) or not caller_device_ura.startswith("easynet:///"):
            message = f"{prefix}: caller_device_ura must be canonical"
            check["errors"].append(message)
            add_error(item_id, message)
        if not isinstance(provider_device_ura, str) or not provider_device_ura.startswith("easynet:///"):
            message = f"{prefix}: provider_device_ura must be canonical"
            check["errors"].append(message)
            add_error(item_id, message)
        if caller_device_ura == provider_device_ura:
            message = f"{prefix}: caller/provider device URAs must be distinct"
            check["errors"].append(message)
            add_error(item_id, message)
        if not isinstance(selected_resource_ura, str) or not selected_resource_ura.startswith("easynet:///"):
            message = f"{prefix}: selected_resource_ura must be canonical"
            check["errors"].append(message)
            add_error(item_id, message)
        if not isinstance(session_id, str) or not session_id:
            message = f"{prefix}: session_id must be set"
            check["errors"].append(message)
            add_error(item_id, message)
        if not positive_int(scenario.get("frames_captured")):
            message = f"{prefix}: frames_captured must be positive"
            check["errors"].append(message)
            add_error(item_id, message)
        if not positive_int(scenario.get("frames_rendered")):
            message = f"{prefix}: frames_rendered must be positive"
            check["errors"].append(message)
            add_error(item_id, message)
        if scenario.get("input_policy_mode") not in {"interactive", "view_only", "policy_blocked"}:
            message = f"{prefix}: input_policy_mode must be interactive, view_only, or policy_blocked"
            check["errors"].append(message)
            add_error(item_id, message)

    observed_targets = sorted(seen_targets)
    check["observed_cross_device_remoteapp_targets"] = observed_targets
    missing_targets = sorted(required_targets - seen_targets)
    if missing_targets:
        message = "cross-device RemoteApp scenarios missing targets: " + ", ".join(missing_targets)
        check["errors"].append(message)
        add_error(item_id, message)

for item in required:
    item_id = item["id"]
    env_name = item["env"]
    report_path_text = os.environ.get(env_name, "")
    check = {
        "id": item_id,
        "env": env_name,
        "report_json": report_path_text or None,
        "status": "missing",
        "coverage": {},
        "errors": [],
    }
    if not report_path_text:
        message = f"missing required report env {env_name}"
        check["errors"].append(message)
        add_error(item_id, message)
        checks.append(check)
        continue
    report_path = pathlib.Path(report_path_text)
    if not report_path.exists():
        message = f"report path does not exist: {report_path}"
        check["errors"].append(message)
        add_error(item_id, message)
        checks.append(check)
        continue
    try:
        report = json.loads(report_path.read_text(encoding="utf-8"))
    except Exception as exc:
        message = f"invalid report JSON: {exc}"
        check["errors"].append(message)
        add_error(item_id, message)
        checks.append(check)
        continue
    status = report.get("status")
    check["status"] = status
    if status != "passed":
        message = f"status is {status!r}, expected 'passed'"
        check["errors"].append(message)
        add_error(item_id, message)
    expected_script = item.get("expected_script")
    check["expected_script"] = expected_script
    check["observed_script"] = report.get("script")
    if expected_script and report.get("script") != expected_script:
        message = (
            f"report script is {report.get('script')!r}, "
            f"expected {expected_script!r}"
        )
        check["errors"].append(message)
        add_error(item_id, message)
    expected_target_kind = item.get("expected_target_kind")
    if expected_target_kind:
        check["expected_target_kind"] = expected_target_kind
        check["observed_target_kind"] = report.get("target_kind")
        if report.get("target_kind") != expected_target_kind:
            message = (
                f"target_kind is {report.get('target_kind')!r}, "
                f"expected {expected_target_kind!r}"
            )
            check["errors"].append(message)
            add_error(item_id, message)
    child_claim = report.get("product_complete_claim")
    if child_claim is True:
        message = "child verifier must not claim product completion"
        check["errors"].append(message)
        add_error(item_id, message)
    coverage = report.get("coverage") if isinstance(report.get("coverage"), dict) else {}
    check["coverage"] = coverage
    for key in item.get("coverage_keys", []):
        if coverage.get(key) is not True:
            message = f"coverage.{key} is not true"
            check["errors"].append(message)
            add_error(item_id, message)
    for expected in item.get("evidence_contract_contains", []):
        evidence_contract = report.get("evidence_contract")
        if not isinstance(evidence_contract, list) or expected not in evidence_contract:
            message = f"evidence_contract missing {expected!r}"
            check["errors"].append(message)
            add_error(item_id, message)
    if item.get("requires_network_route_scenarios"):
        validate_network_route_scenarios(item_id, check, report)
    if item.get("requires_cross_device_remoteapp_scenarios"):
        validate_cross_device_remoteapp_scenarios(item_id, check, report)
    if item.get("requires_platforms_passed"):
        platform_entries = report.get("platforms")
        check["required_passed_platforms"] = item["requires_platforms_passed"]
        if not isinstance(platform_entries, list):
            message = "platforms summary must be a list"
            check["errors"].append(message)
            add_error(item_id, message)
        else:
            platform_by_name = {
                entry.get("platform"): entry
                for entry in platform_entries
                if isinstance(entry, dict) and isinstance(entry.get("platform"), str)
            }
            check["observed_platforms"] = sorted(platform_by_name)
            for platform_name in item["requires_platforms_passed"]:
                platform = platform_by_name.get(platform_name)
                if not isinstance(platform, dict):
                    message = f"platforms.{platform_name} summary is missing"
                    check["errors"].append(message)
                    add_error(item_id, message)
                    continue
                unsupported_targets = platform.get("unsupported_targets")
                if isinstance(unsupported_targets, list) and unsupported_targets:
                    message = f"platforms.{platform_name}.unsupported_targets must be empty"
                    check["errors"].append(message)
                    add_error(item_id, message)
                platform_status = platform.get("status")
                if platform_status is not None and platform_status != "passed":
                    message = f"platforms.{platform_name}.status is {platform_status!r}, expected 'passed'"
                    check["errors"].append(message)
                    add_error(item_id, message)
                required_targets = item.get("requires_passed_targets")
                if required_targets:
                    passed_targets = platform.get("passed_targets")
                    if not isinstance(passed_targets, list) or set(passed_targets) != set(required_targets):
                        message = (
                            f"platforms.{platform_name}.passed_targets is {passed_targets!r}, "
                            f"expected {required_targets!r}"
                        )
                        check["errors"].append(message)
                        add_error(item_id, message)
    if item.get("required_steps"):
        steps = report.get("steps") if isinstance(report.get("steps"), list) else []
        passed_steps = {
            step.get("name")
            for step in steps
            if isinstance(step, dict) and step.get("status") == "passed"
        }
        check["required_steps"] = item["required_steps"]
        check["passed_steps"] = sorted(name for name in passed_steps if isinstance(name, str))
        for step_name in item["required_steps"]:
            if step_name not in passed_steps:
                message = f"required product-flow step {step_name!r} did not pass"
                check["errors"].append(message)
                add_error(item_id, message)
    if item.get("product_flow_step_artifacts"):
        product_flow_root = report_path.parent
        artifact_checks = []
        for step_spec in item["product_flow_step_artifacts"]:
            step_name = step_spec["name"]
            step_dir = product_flow_root / step_name
            result_path = step_dir / "result.json"
            artifact_check = {
                "name": step_name,
                "result_json": str(result_path),
                "errors": [],
            }
            if not result_path.exists():
                message = f"product-flow step result_json path does not exist: {result_path}"
                artifact_check["errors"].append(message)
                check["errors"].append(message)
                add_error(item_id, message)
                artifact_checks.append(artifact_check)
                continue
            try:
                step_result = json.loads(result_path.read_text(encoding="utf-8"))
            except Exception as exc:
                message = f"invalid product-flow step result JSON {result_path}: {exc}"
                artifact_check["errors"].append(message)
                check["errors"].append(message)
                add_error(item_id, message)
                artifact_checks.append(artifact_check)
                continue
            artifact_check["result_status"] = step_result.get("status")
            artifact_check["result_name"] = step_result.get("name")
            if step_result.get("status") != "passed":
                message = f"product-flow step {step_name!r} result status is {step_result.get('status')!r}, expected 'passed'"
                artifact_check["errors"].append(message)
                check["errors"].append(message)
                add_error(item_id, message)
            if step_result.get("name") != step_name:
                message = f"product-flow step result name is {step_result.get('name')!r}, expected {step_name!r}"
                artifact_check["errors"].append(message)
                check["errors"].append(message)
                add_error(item_id, message)
            report_file = step_spec.get("report_json")
            if report_file:
                subreport_path = step_dir / report_file
                artifact_check["subreport_json"] = str(subreport_path)
                if not subreport_path.exists():
                    message = f"product-flow subreport path does not exist: {subreport_path}"
                    artifact_check["errors"].append(message)
                    check["errors"].append(message)
                    add_error(item_id, message)
                else:
                    try:
                        subreport = json.loads(subreport_path.read_text(encoding="utf-8"))
                    except Exception as exc:
                        message = f"invalid product-flow subreport JSON {subreport_path}: {exc}"
                        artifact_check["errors"].append(message)
                        check["errors"].append(message)
                        add_error(item_id, message)
                        subreport = None
                    if isinstance(subreport, dict):
                        artifact_check["subreport_status"] = subreport.get("status")
                        if subreport.get("status") != "passed":
                            message = f"product-flow subreport {step_name!r} status is {subreport.get('status')!r}, expected 'passed'"
                            artifact_check["errors"].append(message)
                            check["errors"].append(message)
                            add_error(item_id, message)
                        expected_step_script = step_spec.get("expected_script")
                        artifact_check["expected_script"] = expected_step_script
                        artifact_check["observed_script"] = subreport.get("script")
                        if expected_step_script and subreport.get("script") != expected_step_script:
                            message = (
                                f"product-flow subreport {step_name!r} script is "
                                f"{subreport.get('script')!r}, expected {expected_step_script!r}"
                            )
                            artifact_check["errors"].append(message)
                            check["errors"].append(message)
                            add_error(item_id, message)
                        expected_step_target_kind = step_spec.get("expected_target_kind")
                        if expected_step_target_kind:
                            artifact_check["expected_target_kind"] = expected_step_target_kind
                            artifact_check["observed_target_kind"] = subreport.get("target_kind")
                            if subreport.get("target_kind") != expected_step_target_kind:
                                message = (
                                    f"product-flow subreport {step_name!r} target_kind is "
                                    f"{subreport.get('target_kind')!r}, expected {expected_step_target_kind!r}"
                                )
                                artifact_check["errors"].append(message)
                                check["errors"].append(message)
                                add_error(item_id, message)
                        if subreport.get("product_complete_claim") is True:
                            message = f"product-flow subreport {step_name!r} must not claim product completion"
                            artifact_check["errors"].append(message)
                            check["errors"].append(message)
                            add_error(item_id, message)
                        if step_spec.get("requires_evidence_json"):
                            step_evidence_json = subreport.get("evidence_json")
                            artifact_check["evidence_json"] = step_evidence_json if isinstance(step_evidence_json, str) else None
                            if not isinstance(step_evidence_json, str) or not step_evidence_json.strip():
                                message = f"product-flow subreport {step_name!r} evidence_json must be set"
                                artifact_check["errors"].append(message)
                                check["errors"].append(message)
                                add_error(item_id, message)
                            else:
                                step_evidence_path = pathlib.Path(step_evidence_json)
                                if not step_evidence_path.is_absolute():
                                    step_evidence_path = subreport_path.parent / step_evidence_path
                                artifact_check["resolved_evidence_json"] = str(step_evidence_path)
                                if not step_evidence_path.exists():
                                    message = f"product-flow subreport evidence_json path does not exist: {step_evidence_path}"
                                    artifact_check["errors"].append(message)
                                    check["errors"].append(message)
                                    add_error(item_id, message)
                                else:
                                    before_count = len(check["errors"])
                                    read_required_evidence_json(
                                        item_id,
                                        check,
                                        step_evidence_path,
                                        f"product-flow subreport {step_name!r}",
                                    )
                                    if len(check["errors"]) > before_count:
                                        artifact_check["errors"].extend(check["errors"][before_count:])
                        if step_spec.get("cross_device"):
                            topology = subreport.get("topology") if isinstance(subreport.get("topology"), dict) else {}
                            step_coverage = subreport.get("coverage") if isinstance(subreport.get("coverage"), dict) else {}
                            observed_device_pairs = topology.get("observed_device_pairs")
                            if topology.get("requires_distinct_devices") is not True:
                                message = f"product-flow cross-device subreport {step_name!r} requires_distinct_devices is not true"
                                artifact_check["errors"].append(message)
                                check["errors"].append(message)
                                add_error(item_id, message)
                            if topology.get("distinct_device_uras_observed") is not True:
                                message = f"product-flow cross-device subreport {step_name!r} distinct_device_uras_observed is not true"
                                artifact_check["errors"].append(message)
                                check["errors"].append(message)
                                add_error(item_id, message)
                            if topology.get("local_provider_boundary_only") is not False:
                                message = f"product-flow cross-device subreport {step_name!r} local_provider_boundary_only is not false"
                                artifact_check["errors"].append(message)
                                check["errors"].append(message)
                                add_error(item_id, message)
                            if step_coverage.get("local_provider_boundary_only") is not False:
                                message = f"product-flow cross-device subreport {step_name!r} coverage.local_provider_boundary_only is not false"
                                artifact_check["errors"].append(message)
                                check["errors"].append(message)
                                add_error(item_id, message)
                            if not isinstance(observed_device_pairs, list) or not observed_device_pairs:
                                message = f"product-flow cross-device subreport {step_name!r} observed_device_pairs must not be empty"
                                artifact_check["errors"].append(message)
                                check["errors"].append(message)
                                add_error(item_id, message)
        check["product_flow_step_artifacts"] = artifact_checks
    if item.get("requires_evidence_json"):
        evidence_json = report.get("evidence_json")
        check["evidence_json"] = evidence_json if isinstance(evidence_json, str) else None
        if not isinstance(evidence_json, str) or not evidence_json.strip():
            message = "evidence_json must name a required live evidence artifact"
            check["errors"].append(message)
            add_error(item_id, message)
        else:
            evidence_path = pathlib.Path(evidence_json)
            if not evidence_path.is_absolute():
                evidence_path = report_path.parent / evidence_path
            check["resolved_evidence_json"] = str(evidence_path)
            if not evidence_path.exists():
                message = f"evidence_json path does not exist: {evidence_path}"
                check["errors"].append(message)
                add_error(item_id, message)
            else:
                read_required_evidence_json(
                    item_id,
                    check,
                    evidence_path,
                    f"report {item_id!r}",
                )
    if item.get("cross_device"):
        topology = report.get("topology") if isinstance(report.get("topology"), dict) else {}
        check["topology"] = topology
        if topology.get("requires_distinct_devices") is not True:
            message = "topology.requires_distinct_devices is not true"
            check["errors"].append(message)
            add_error(item_id, message)
        if topology.get("distinct_device_uras_observed") is not True:
            message = "topology.distinct_device_uras_observed is not true"
            check["errors"].append(message)
            add_error(item_id, message)
        if topology.get("local_provider_boundary_only") is not False:
            message = "topology.local_provider_boundary_only is not false"
            check["errors"].append(message)
            add_error(item_id, message)
        if coverage.get("local_provider_boundary_only") is not False:
            message = "coverage.local_provider_boundary_only is not false"
            check["errors"].append(message)
            add_error(item_id, message)
        if item.get("requires_observed_device_pairs"):
            observed_device_pairs = topology.get("observed_device_pairs")
            if not isinstance(observed_device_pairs, list) or not observed_device_pairs:
                message = "topology.observed_device_pairs must not be empty"
                check["errors"].append(message)
                add_error(item_id, message)
            else:
                distinct_pairs = []
                for index, pair in enumerate(observed_device_pairs):
                    if not isinstance(pair, dict):
                        message = f"topology.observed_device_pairs[{index}] must be an object"
                        check["errors"].append(message)
                        add_error(item_id, message)
                        continue
                    caller_ura = pair.get("caller_ura")
                    provider_ura = pair.get("provider_ura")
                    distinct = pair.get("distinct_device_uras")
                    if not isinstance(caller_ura, str) or not caller_ura:
                        message = f"topology.observed_device_pairs[{index}].caller_ura must be set"
                        check["errors"].append(message)
                        add_error(item_id, message)
                    if not isinstance(provider_ura, str) or not provider_ura:
                        message = f"topology.observed_device_pairs[{index}].provider_ura must be set"
                        check["errors"].append(message)
                        add_error(item_id, message)
                    if distinct is not True or caller_ura == provider_ura:
                        message = f"topology.observed_device_pairs[{index}] is not a distinct device pair"
                        check["errors"].append(message)
                        add_error(item_id, message)
                    else:
                        distinct_pairs.append(pair)
                check["observed_distinct_device_pair_count"] = len(distinct_pairs)
    checks.append(check)

effective_status = "failed" if errors else "passed"
report = {
    "script": "tools/scripts/remoteapp-product-completion-e2e.sh",
    "status": effective_status,
    "mode": mode,
    "reason": "all product-completion evidence passed" if not errors else "missing or failed required product evidence",
    "product_complete_claim": effective_status == "passed",
    "required_evidence_count": len(required),
    "checks": checks,
    "errors": errors,
    "non_claims_when_failed": [
        "a failed or missing report means RemoteApp product completion is unproven",
        "self-tests and source checks are not live product evidence",
        "local-provider-only topology is not cross-device product evidence",
    ],
}
(out_dir / "report.json").write_text(
    json.dumps(report, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
(out_dir / "report.md").write_text(
    "# RemoteApp Product Completion E2E\n\n"
    f"- Status: `{effective_status}`\n"
    f"- Product complete claim: `{str(effective_status == 'passed').lower()}`\n"
    f"- Required evidence count: `{len(required)}`\n"
    f"- Reason: `{report['reason']}`\n",
    encoding="utf-8",
)
if errors:
    for error in errors:
        print(error, file=sys.stderr)
    raise SystemExit(1)
PY
}

write_synthetic_report() {
  local path="$1"
  local id="$2"
  python3 - "$path" "$id" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
item_id = sys.argv[2]
coverage_by_id = {
    "cross_device_smoke": {
        "cross_device_hub_routing": True,
        "synthetic_stream_bidi_carrier": True,
        "distinct_device_uras_observed": True,
        "local_provider_boundary_only": False,
    },
    "cross_device_remoteapp": {
        "remoteapp_cross_device_session": True,
        "display": True,
        "window": True,
        "application": True,
        "remote_media_rendered": True,
        "input_policy_checked": True,
        "distinct_device_uras_observed": True,
        "local_provider_boundary_only": False,
    },
    "cross_platform_capture": {"macos": True, "windows": True, "linux": True},
    "input_injection": {"macos": True, "windows": True, "linux": True},
    "media_adaptation": {"baseline": True, "degraded_network": True, "backpressure": True},
    "multi_window_tracking": {
        "independent_window_streams": True,
        "geometry_churn": True,
        "application_window_set_churn": True,
        "target_loss_rebind": True,
        "multi_display_application": True,
    },
    "network_fallback": {
        "direct": True,
        "stun_srflx": True,
        "turn_relay": True,
        "easynet_relay": True,
    },
    "crash_restart_recovery": {
        "daemon_restart_active_session": True,
        "plugin_worker_restart": True,
        "terminal_receipt_replay_after_crash": True,
        "stale_socket_restart_cleanup": True,
    },
}
script_by_id = {
    "frontend_product_flow": "tools/scripts/frontend-remoteapp-product-flow-e2e.sh",
    "browser_lifecycle": "tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh",
    "cross_device_smoke": "tools/scripts/remoteapp-cross-device-product-smoke.sh",
    "cross_device_remoteapp": "tools/scripts/remoteapp-cross-device-remoteapp-e2e.sh",
    "cross_platform_capture": "tools/scripts/remoteapp-cross-platform-capture-e2e.sh",
    "input_injection": "tools/scripts/remoteapp-input-injection-e2e.sh",
    "media_adaptation": "tools/scripts/remoteapp-media-adaptation-e2e.sh",
    "multi_window_tracking": "tools/scripts/remoteapp-multi-window-tracking-e2e.sh",
    "network_fallback": "tools/scripts/remoteapp-network-fallback-e2e.sh",
    "session_timeout_window": "tools/scripts/host-remoteapp-session-timeout-e2e.sh",
    "session_timeout_application": "tools/scripts/host-remoteapp-session-timeout-e2e.sh",
    "session_cancel_window": "tools/scripts/host-remoteapp-session-cancel-e2e.sh",
    "session_cancel_application": "tools/scripts/host-remoteapp-session-cancel-e2e.sh",
    "permission_revoke_window": "tools/scripts/host-remoteapp-permission-revoke-e2e.sh",
    "permission_revoke_application": "tools/scripts/host-remoteapp-permission-revoke-e2e.sh",
    "session_resume_window": "tools/scripts/host-remoteapp-session-resume-e2e.sh",
    "session_resume_application": "tools/scripts/host-remoteapp-session-resume-e2e.sh",
    "crash_restart_recovery": "tools/scripts/remoteapp-crash-restart-recovery-e2e.sh",
}
evidence_json_ids = {
    "browser_lifecycle",
    "cross_device_remoteapp",
    "cross_platform_capture",
    "input_injection",
    "media_adaptation",
    "multi_window_tracking",
    "network_fallback",
    "session_timeout_window",
    "session_timeout_application",
    "session_cancel_window",
    "session_cancel_application",
    "permission_revoke_window",
    "permission_revoke_application",
    "session_resume_window",
    "session_resume_application",
    "crash_restart_recovery",
}
lifecycle_target_by_id = {
    "session_timeout_window": "window",
    "session_timeout_application": "application",
    "session_cancel_window": "window",
    "session_cancel_application": "application",
    "permission_revoke_window": "window",
    "permission_revoke_application": "application",
    "session_resume_window": "window",
    "session_resume_application": "application",
}
report = {
    "script": script_by_id[item_id],
    "status": "passed",
    "product_complete_claim": False,
    "coverage": coverage_by_id.get(item_id, {}),
}
if item_id == "frontend_product_flow":
    report["target_kind"] = "both"
    report["evidence_contract"] = [
        "Browser/Tauri RemoteApp lifecycle evidence",
        "cross-device product smoke with distinct device URAs",
    ]
    report["steps"] = [
        {"name": "hub-api-readiness-preflight", "status": "passed"},
        {"name": "product-runtime-readiness-preflight", "status": "passed"},
        {"name": "frontend-typecheck", "status": "passed"},
        {"name": "frontend-remoteapp-ui-flow", "status": "passed"},
        {"name": "frontend-browser-lifecycle", "status": "passed"},
        {"name": "cross-device-product-smoke", "status": "passed"},
        {"name": "host-permission-subject", "status": "passed"},
        {"name": "host-target-picker-freshness", "status": "passed"},
        {"name": "host-decoded-frame-window", "status": "passed"},
        {"name": "host-decoded-frame-application", "status": "passed"},
        {"name": "host-view-only-input-window", "status": "passed"},
        {"name": "host-view-only-input-application", "status": "passed"},
    ]
if item_id in lifecycle_target_by_id:
    report["target_kind"] = lifecycle_target_by_id[item_id]
if item_id == "cross_platform_capture":
    report["platforms"] = [
        {
            "platform": platform,
            "passed_targets": ["application", "display", "window"],
            "unsupported_targets": [],
        }
        for platform in ("linux", "macos", "windows")
    ]
if item_id == "input_injection":
    report["platforms"] = [
        {"platform": platform, "status": "passed"}
        for platform in ("linux", "macos", "windows")
    ]
if item_id == "network_fallback":
    network_routes = [
        ("direct", "connected", "direct", ["host"], ["direct"], ["relay"]),
        ("stun_srflx", "connected", "stun_srflx", ["srflx", "host"], ["stun_srflx"], ["direct"]),
        ("turn_relay", "completed", "relay", ["relay", "host"], ["relay"], ["direct", "stun_srflx"]),
        ("easynet_relay", "completed", "relay", ["relay", "relay"], ["relay"], ["direct", "stun_srflx"]),
    ]
    report["scenario_count"] = len(network_routes)
    report["scenarios"] = [
        {
            "route_kind": route_kind,
            "ice_connection_state": ice_connection_state,
            "selected_route_class": selected_route_class,
            "candidate_pair_id": f"pair-{route_kind}",
            "candidate_types": candidate_types,
            "allowed_route_classes": allowed_route_classes,
            "blocked_route_classes": blocked_route_classes,
            "frames_rendered": 8,
            "session_id": f"rd-product-network-{route_kind}",
        }
        for (
            route_kind,
            ice_connection_state,
            selected_route_class,
            candidate_types,
            allowed_route_classes,
            blocked_route_classes,
        ) in network_routes
    ]
if item_id == "cross_device_smoke":
    report["topology"] = {
        "requires_distinct_devices": True,
        "observed_device_pairs": [
            {
                "step": "cross-device-routing",
                "caller_ura": "easynet:///r/localhost/device/synthetic-caller",
                "provider_ura": "easynet:///r/localhost/device/synthetic-provider",
                "distinct_device_uras": True,
            }
        ],
        "distinct_device_uras_observed": True,
        "local_provider_boundary_only": False,
    }
if item_id == "cross_device_remoteapp":
    caller = "easynet:///r/localhost/device/synthetic-caller"
    provider = "easynet:///r/localhost/device/synthetic-provider"
    report["topology"] = {
        "requires_distinct_devices": True,
        "observed_device_pairs": [
            {
                "step": "cross-device-remoteapp",
                "caller_ura": caller,
                "provider_ura": provider,
                "distinct_device_uras": True,
            }
        ],
        "distinct_device_uras_observed": True,
        "local_provider_boundary_only": False,
    }
    report["scenario_count"] = 3
    report["scenarios"] = [
        {
            "target_kind": target_kind,
            "caller_device_ura": caller,
            "provider_device_ura": provider,
            "selected_resource_ura": f"easynet:///r/localhost/resource/device.synthetic-provider/{target_kind}.selected",
            "session_id": f"rd-product-cross-device-{target_kind}",
            "frames_captured": 12,
            "frames_rendered": 10,
            "input_policy_mode": "view_only",
        }
        for target_kind in ("display", "window", "application")
    ]
path.parent.mkdir(parents=True, exist_ok=True)
if item_id == "frontend_product_flow":
    for step in report["steps"]:
        step_name = step["name"]
        step_dir = path.parent / step_name
        step_dir.mkdir(parents=True, exist_ok=True)
        (step_dir / "result.json").write_text(
            json.dumps({"name": step_name, "status": "passed"}, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        if step_name == "frontend-browser-lifecycle":
            evidence_path = step_dir / "evidence.json"
            evidence_path.write_text(
                json.dumps({"status": "passed", "synthetic": True, "step": step_name}, indent=2, sort_keys=True) + "\n",
                encoding="utf-8",
            )
            (step_dir / "report.json").write_text(
                json.dumps({
                    "script": "tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh",
                    "status": "passed",
                    "product_complete_claim": False,
                    "evidence_json": str(evidence_path),
                }, indent=2, sort_keys=True) + "\n",
                encoding="utf-8",
            )
        elif step_name == "cross-device-product-smoke":
            (step_dir / "evidence-report.json").write_text(
                json.dumps({
                    "script": "tools/scripts/remoteapp-cross-device-product-smoke.sh",
                    "status": "passed",
                    "product_complete_claim": False,
                    "topology": {
                        "requires_distinct_devices": True,
                        "observed_device_pairs": [
                            {
                                "step": "cross-device-routing",
                                "caller_ura": "easynet:///r/localhost/device/synthetic-caller",
                                "provider_ura": "easynet:///r/localhost/device/synthetic-provider",
                                "distinct_device_uras": True,
                            }
                        ],
                        "distinct_device_uras_observed": True,
                        "local_provider_boundary_only": False,
                    },
                    "coverage": {
                        "cross_device_hub_routing": True,
                        "synthetic_stream_bidi_carrier": True,
                        "distinct_device_uras_observed": True,
                        "local_provider_boundary_only": False,
                    },
                }, indent=2, sort_keys=True) + "\n",
                encoding="utf-8",
            )
        elif step_name.startswith("host-"):
            evidence_path = step_dir / "evidence.json"
            evidence_path.write_text(
                json.dumps({"status": "passed", "synthetic": True, "step": step_name}, indent=2, sort_keys=True) + "\n",
                encoding="utf-8",
            )
            script_by_step = {
                "host-permission-subject": "tools/scripts/host-remoteapp-permission-subject-e2e.sh",
                "host-target-picker-freshness": "tools/scripts/host-remoteapp-target-picker-freshness-e2e.sh",
                "host-decoded-frame-window": "tools/scripts/host-remoteapp-decoded-frame-e2e.sh",
                "host-decoded-frame-application": "tools/scripts/host-remoteapp-decoded-frame-e2e.sh",
                "host-view-only-input-window": "tools/scripts/host-remoteapp-view-only-input-safety-e2e.sh",
                "host-view-only-input-application": "tools/scripts/host-remoteapp-view-only-input-safety-e2e.sh",
            }
            target_kind_by_step = {
                "host-decoded-frame-window": "window",
                "host-decoded-frame-application": "application",
                "host-view-only-input-window": "window",
                "host-view-only-input-application": "application",
            }
            report_payload = {
                "script": script_by_step[step_name],
                "status": "passed",
                "product_complete_claim": False,
                "evidence_json": str(evidence_path),
            }
            if step_name in target_kind_by_step:
                report_payload["target_kind"] = target_kind_by_step[step_name]
            (step_dir / "report.json").write_text(
                json.dumps(report_payload, indent=2, sort_keys=True) + "\n",
                encoding="utf-8",
            )
if item_id in evidence_json_ids:
    evidence_path = path.with_suffix(".evidence.json")
    evidence_path.write_text(
        json.dumps({"status": "passed", "synthetic": True, "report_id": item_id}, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    report["evidence_json"] = str(evidence_path)
path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
}

run_self_test() {
  self_test_tmp="$(mktemp -d)"
  trap 'if [[ -n "${self_test_tmp:-}" ]]; then rm -rf "$self_test_tmp"; fi' EXIT
  local tmp="$self_test_tmp"
  local ids=(
    frontend_product_flow
    browser_lifecycle
    cross_device_smoke
    cross_device_remoteapp
    cross_platform_capture
    input_injection
    media_adaptation
    multi_window_tracking
    network_fallback
    session_timeout_window
    session_timeout_application
    session_cancel_window
    session_cancel_application
    permission_revoke_window
    permission_revoke_application
    session_resume_window
    session_resume_application
    crash_restart_recovery
  )
  for id in "${ids[@]}"; do
    write_synthetic_report "$tmp/$id.json" "$id"
  done
  export EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_TIMEOUT_WINDOW_REPORT_JSON="$tmp/session_timeout_window.json"
  export EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_TIMEOUT_APPLICATION_REPORT_JSON="$tmp/session_timeout_application.json"
  export EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_CANCEL_WINDOW_REPORT_JSON="$tmp/session_cancel_window.json"
  export EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_CANCEL_APPLICATION_REPORT_JSON="$tmp/session_cancel_application.json"
  export EASYNET_REMOTEAPP_PRODUCT_COMPLETION_PERMISSION_REVOKE_WINDOW_REPORT_JSON="$tmp/permission_revoke_window.json"
  export EASYNET_REMOTEAPP_PRODUCT_COMPLETION_PERMISSION_REVOKE_APPLICATION_REPORT_JSON="$tmp/permission_revoke_application.json"
  export EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_RESUME_WINDOW_REPORT_JSON="$tmp/session_resume_window.json"
  export EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_RESUME_APPLICATION_REPORT_JSON="$tmp/session_resume_application.json"
  export EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_REMOTEAPP_REPORT_JSON="$tmp/cross_device_remoteapp.json"

  env \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_FRONTEND_PRODUCT_FLOW_REPORT_JSON="$tmp/frontend_product_flow.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_BROWSER_LIFECYCLE_REPORT_JSON="$tmp/browser_lifecycle.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_SMOKE_REPORT_JSON="$tmp/cross_device_smoke.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_PLATFORM_CAPTURE_REPORT_JSON="$tmp/cross_platform_capture.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_INPUT_INJECTION_REPORT_JSON="$tmp/input_injection.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MEDIA_ADAPTATION_REPORT_JSON="$tmp/media_adaptation.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MULTI_WINDOW_TRACKING_REPORT_JSON="$tmp/multi_window_tracking.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_NETWORK_FALLBACK_REPORT_JSON="$tmp/network_fallback.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CRASH_RESTART_RECOVERY_REPORT_JSON="$tmp/crash_restart_recovery.json" \
    "$0" --check --out-dir "$tmp/pass" >/dev/null
  grep -q '"product_complete_claim": true' "$tmp/pass/report.json"

  python3 - "$tmp/session_timeout_application.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["target_kind"] = "window"
path.write_text(json.dumps(report) + "\n", encoding="utf-8")
PY
  if env \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_FRONTEND_PRODUCT_FLOW_REPORT_JSON="$tmp/frontend_product_flow.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_BROWSER_LIFECYCLE_REPORT_JSON="$tmp/browser_lifecycle.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_SMOKE_REPORT_JSON="$tmp/cross_device_smoke.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_PLATFORM_CAPTURE_REPORT_JSON="$tmp/cross_platform_capture.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_INPUT_INJECTION_REPORT_JSON="$tmp/input_injection.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MEDIA_ADAPTATION_REPORT_JSON="$tmp/media_adaptation.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MULTI_WINDOW_TRACKING_REPORT_JSON="$tmp/multi_window_tracking.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_NETWORK_FALLBACK_REPORT_JSON="$tmp/network_fallback.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CRASH_RESTART_RECOVERY_REPORT_JSON="$tmp/crash_restart_recovery.json" \
    "$0" --check --out-dir "$tmp/wrong-lifecycle-target-kind" >/dev/null 2>&1; then
    echo "self-test accepted wrong lifecycle target_kind" >&2
    exit 1
  fi
  write_synthetic_report "$tmp/session_timeout_application.json" session_timeout_application

  python3 - "$tmp/frontend_product_flow.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["steps"] = [
    step for step in report["steps"]
    if step.get("name") != "frontend-browser-lifecycle"
]
path.write_text(json.dumps(report) + "\n", encoding="utf-8")
PY
  if env \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_FRONTEND_PRODUCT_FLOW_REPORT_JSON="$tmp/frontend_product_flow.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_BROWSER_LIFECYCLE_REPORT_JSON="$tmp/browser_lifecycle.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_SMOKE_REPORT_JSON="$tmp/cross_device_smoke.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_PLATFORM_CAPTURE_REPORT_JSON="$tmp/cross_platform_capture.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_INPUT_INJECTION_REPORT_JSON="$tmp/input_injection.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MEDIA_ADAPTATION_REPORT_JSON="$tmp/media_adaptation.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MULTI_WINDOW_TRACKING_REPORT_JSON="$tmp/multi_window_tracking.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_NETWORK_FALLBACK_REPORT_JSON="$tmp/network_fallback.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CRASH_RESTART_RECOVERY_REPORT_JSON="$tmp/crash_restart_recovery.json" \
    "$0" --check --out-dir "$tmp/missing-product-flow-step" >/dev/null 2>&1; then
    echo "self-test accepted missing frontend product-flow step" >&2
    exit 1
  fi
  write_synthetic_report "$tmp/frontend_product_flow.json" frontend_product_flow

  python3 - "$tmp/frontend_product_flow.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["target_kind"] = "window"
path.write_text(json.dumps(report) + "\n", encoding="utf-8")
PY
  if env \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_FRONTEND_PRODUCT_FLOW_REPORT_JSON="$tmp/frontend_product_flow.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_BROWSER_LIFECYCLE_REPORT_JSON="$tmp/browser_lifecycle.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_SMOKE_REPORT_JSON="$tmp/cross_device_smoke.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_PLATFORM_CAPTURE_REPORT_JSON="$tmp/cross_platform_capture.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_INPUT_INJECTION_REPORT_JSON="$tmp/input_injection.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MEDIA_ADAPTATION_REPORT_JSON="$tmp/media_adaptation.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MULTI_WINDOW_TRACKING_REPORT_JSON="$tmp/multi_window_tracking.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_NETWORK_FALLBACK_REPORT_JSON="$tmp/network_fallback.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CRASH_RESTART_RECOVERY_REPORT_JSON="$tmp/crash_restart_recovery.json" \
    "$0" --check --out-dir "$tmp/product-flow-window-only" >/dev/null 2>&1; then
    echo "self-test accepted product-flow target_kind other than both" >&2
    exit 1
  fi
  write_synthetic_report "$tmp/frontend_product_flow.json" frontend_product_flow

  python3 - "$tmp/frontend_product_flow.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
result_path = path.parent / "host-decoded-frame-window" / "result.json"
result_path.unlink()
path.write_text(json.dumps(report) + "\n", encoding="utf-8")
PY
  if env \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_FRONTEND_PRODUCT_FLOW_REPORT_JSON="$tmp/frontend_product_flow.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_BROWSER_LIFECYCLE_REPORT_JSON="$tmp/browser_lifecycle.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_SMOKE_REPORT_JSON="$tmp/cross_device_smoke.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_PLATFORM_CAPTURE_REPORT_JSON="$tmp/cross_platform_capture.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_INPUT_INJECTION_REPORT_JSON="$tmp/input_injection.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MEDIA_ADAPTATION_REPORT_JSON="$tmp/media_adaptation.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MULTI_WINDOW_TRACKING_REPORT_JSON="$tmp/multi_window_tracking.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_NETWORK_FALLBACK_REPORT_JSON="$tmp/network_fallback.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CRASH_RESTART_RECOVERY_REPORT_JSON="$tmp/crash_restart_recovery.json" \
    "$0" --check --out-dir "$tmp/missing-product-flow-step-result" >/dev/null 2>&1; then
    echo "self-test accepted missing product-flow step result artifact" >&2
    exit 1
  fi
  write_synthetic_report "$tmp/frontend_product_flow.json" frontend_product_flow

  python3 - "$tmp/frontend_product_flow.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
subreport_path = path.parent / "host-view-only-input-application" / "report.json"
subreport = json.loads(subreport_path.read_text(encoding="utf-8"))
pathlib.Path(subreport["evidence_json"]).unlink()
subreport_path.write_text(json.dumps(subreport) + "\n", encoding="utf-8")
path.write_text(json.dumps(report) + "\n", encoding="utf-8")
PY
  if env \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_FRONTEND_PRODUCT_FLOW_REPORT_JSON="$tmp/frontend_product_flow.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_BROWSER_LIFECYCLE_REPORT_JSON="$tmp/browser_lifecycle.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_SMOKE_REPORT_JSON="$tmp/cross_device_smoke.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_PLATFORM_CAPTURE_REPORT_JSON="$tmp/cross_platform_capture.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_INPUT_INJECTION_REPORT_JSON="$tmp/input_injection.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MEDIA_ADAPTATION_REPORT_JSON="$tmp/media_adaptation.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MULTI_WINDOW_TRACKING_REPORT_JSON="$tmp/multi_window_tracking.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_NETWORK_FALLBACK_REPORT_JSON="$tmp/network_fallback.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CRASH_RESTART_RECOVERY_REPORT_JSON="$tmp/crash_restart_recovery.json" \
    "$0" --check --out-dir "$tmp/missing-product-flow-step-evidence" >/dev/null 2>&1; then
    echo "self-test accepted missing product-flow subreport evidence artifact" >&2
    exit 1
  fi
  write_synthetic_report "$tmp/frontend_product_flow.json" frontend_product_flow

  python3 - "$tmp/frontend_product_flow.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
subreport_path = path.parent / "host-target-picker-freshness" / "report.json"
subreport = json.loads(subreport_path.read_text(encoding="utf-8"))
evidence_path = pathlib.Path(subreport["evidence_json"])
evidence = json.loads(evidence_path.read_text(encoding="utf-8"))
evidence["status"] = "failed"
evidence_path.write_text(json.dumps(evidence) + "\n", encoding="utf-8")
subreport_path.write_text(json.dumps(subreport) + "\n", encoding="utf-8")
path.write_text(json.dumps(report) + "\n", encoding="utf-8")
PY
  if env \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_FRONTEND_PRODUCT_FLOW_REPORT_JSON="$tmp/frontend_product_flow.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_BROWSER_LIFECYCLE_REPORT_JSON="$tmp/browser_lifecycle.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_SMOKE_REPORT_JSON="$tmp/cross_device_smoke.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_PLATFORM_CAPTURE_REPORT_JSON="$tmp/cross_platform_capture.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_INPUT_INJECTION_REPORT_JSON="$tmp/input_injection.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MEDIA_ADAPTATION_REPORT_JSON="$tmp/media_adaptation.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MULTI_WINDOW_TRACKING_REPORT_JSON="$tmp/multi_window_tracking.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_NETWORK_FALLBACK_REPORT_JSON="$tmp/network_fallback.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CRASH_RESTART_RECOVERY_REPORT_JSON="$tmp/crash_restart_recovery.json" \
    "$0" --check --out-dir "$tmp/failed-product-flow-subreport-evidence-status" >/dev/null 2>&1; then
    echo "self-test accepted failed product-flow subreport evidence status" >&2
    exit 1
  fi
  write_synthetic_report "$tmp/frontend_product_flow.json" frontend_product_flow

  python3 - "$tmp/frontend_product_flow.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
subreport_path = path.parent / "host-permission-subject" / "report.json"
subreport = json.loads(subreport_path.read_text(encoding="utf-8"))
subreport["script"] = "tools/scripts/wrong-host-permission-subject-e2e.sh"
subreport_path.write_text(json.dumps(subreport) + "\n", encoding="utf-8")
path.write_text(json.dumps(report) + "\n", encoding="utf-8")
PY
  if env \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_FRONTEND_PRODUCT_FLOW_REPORT_JSON="$tmp/frontend_product_flow.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_BROWSER_LIFECYCLE_REPORT_JSON="$tmp/browser_lifecycle.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_SMOKE_REPORT_JSON="$tmp/cross_device_smoke.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_PLATFORM_CAPTURE_REPORT_JSON="$tmp/cross_platform_capture.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_INPUT_INJECTION_REPORT_JSON="$tmp/input_injection.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MEDIA_ADAPTATION_REPORT_JSON="$tmp/media_adaptation.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MULTI_WINDOW_TRACKING_REPORT_JSON="$tmp/multi_window_tracking.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_NETWORK_FALLBACK_REPORT_JSON="$tmp/network_fallback.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CRASH_RESTART_RECOVERY_REPORT_JSON="$tmp/crash_restart_recovery.json" \
    "$0" --check --out-dir "$tmp/wrong-product-flow-host-subreport-script" >/dev/null 2>&1; then
    echo "self-test accepted wrong product-flow host subreport script identity" >&2
    exit 1
  fi
  write_synthetic_report "$tmp/frontend_product_flow.json" frontend_product_flow

  python3 - "$tmp/frontend_product_flow.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
subreport_path = path.parent / "host-decoded-frame-window" / "report.json"
subreport = json.loads(subreport_path.read_text(encoding="utf-8"))
subreport["target_kind"] = "application"
subreport_path.write_text(json.dumps(subreport) + "\n", encoding="utf-8")
path.write_text(json.dumps(report) + "\n", encoding="utf-8")
PY
  if env \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_FRONTEND_PRODUCT_FLOW_REPORT_JSON="$tmp/frontend_product_flow.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_BROWSER_LIFECYCLE_REPORT_JSON="$tmp/browser_lifecycle.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_SMOKE_REPORT_JSON="$tmp/cross_device_smoke.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_PLATFORM_CAPTURE_REPORT_JSON="$tmp/cross_platform_capture.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_INPUT_INJECTION_REPORT_JSON="$tmp/input_injection.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MEDIA_ADAPTATION_REPORT_JSON="$tmp/media_adaptation.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MULTI_WINDOW_TRACKING_REPORT_JSON="$tmp/multi_window_tracking.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_NETWORK_FALLBACK_REPORT_JSON="$tmp/network_fallback.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CRASH_RESTART_RECOVERY_REPORT_JSON="$tmp/crash_restart_recovery.json" \
    "$0" --check --out-dir "$tmp/wrong-product-flow-host-subreport-target-kind" >/dev/null 2>&1; then
    echo "self-test accepted wrong product-flow host subreport target_kind" >&2
    exit 1
  fi
  write_synthetic_report "$tmp/frontend_product_flow.json" frontend_product_flow

  python3 - "$tmp/browser_lifecycle.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
pathlib.Path(report["evidence_json"]).unlink()
path.write_text(json.dumps(report) + "\n", encoding="utf-8")
PY
  if env \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_FRONTEND_PRODUCT_FLOW_REPORT_JSON="$tmp/frontend_product_flow.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_BROWSER_LIFECYCLE_REPORT_JSON="$tmp/browser_lifecycle.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_SMOKE_REPORT_JSON="$tmp/cross_device_smoke.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_PLATFORM_CAPTURE_REPORT_JSON="$tmp/cross_platform_capture.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_INPUT_INJECTION_REPORT_JSON="$tmp/input_injection.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MEDIA_ADAPTATION_REPORT_JSON="$tmp/media_adaptation.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MULTI_WINDOW_TRACKING_REPORT_JSON="$tmp/multi_window_tracking.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_NETWORK_FALLBACK_REPORT_JSON="$tmp/network_fallback.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CRASH_RESTART_RECOVERY_REPORT_JSON="$tmp/crash_restart_recovery.json" \
    "$0" --check --out-dir "$tmp/missing-evidence-json-artifact" >/dev/null 2>&1; then
    echo "self-test accepted missing evidence_json artifact" >&2
    exit 1
  fi
  write_synthetic_report "$tmp/browser_lifecycle.json" browser_lifecycle

  python3 - "$tmp/browser_lifecycle.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
evidence_path = pathlib.Path(report["evidence_json"])
evidence = json.loads(evidence_path.read_text(encoding="utf-8"))
evidence["status"] = "failed"
evidence_path.write_text(json.dumps(evidence) + "\n", encoding="utf-8")
path.write_text(json.dumps(report) + "\n", encoding="utf-8")
PY
  if env \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_FRONTEND_PRODUCT_FLOW_REPORT_JSON="$tmp/frontend_product_flow.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_BROWSER_LIFECYCLE_REPORT_JSON="$tmp/browser_lifecycle.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_SMOKE_REPORT_JSON="$tmp/cross_device_smoke.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_PLATFORM_CAPTURE_REPORT_JSON="$tmp/cross_platform_capture.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_INPUT_INJECTION_REPORT_JSON="$tmp/input_injection.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MEDIA_ADAPTATION_REPORT_JSON="$tmp/media_adaptation.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MULTI_WINDOW_TRACKING_REPORT_JSON="$tmp/multi_window_tracking.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_NETWORK_FALLBACK_REPORT_JSON="$tmp/network_fallback.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CRASH_RESTART_RECOVERY_REPORT_JSON="$tmp/crash_restart_recovery.json" \
    "$0" --check --out-dir "$tmp/failed-evidence-json-status" >/dev/null 2>&1; then
    echo "self-test accepted failed evidence_json status" >&2
    exit 1
  fi
  write_synthetic_report "$tmp/browser_lifecycle.json" browser_lifecycle

  python3 - "$tmp/cross_device_smoke.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["topology"]["observed_device_pairs"] = []
path.write_text(json.dumps(report) + "\n", encoding="utf-8")
PY
  if env \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_FRONTEND_PRODUCT_FLOW_REPORT_JSON="$tmp/frontend_product_flow.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_BROWSER_LIFECYCLE_REPORT_JSON="$tmp/browser_lifecycle.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_SMOKE_REPORT_JSON="$tmp/cross_device_smoke.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_PLATFORM_CAPTURE_REPORT_JSON="$tmp/cross_platform_capture.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_INPUT_INJECTION_REPORT_JSON="$tmp/input_injection.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MEDIA_ADAPTATION_REPORT_JSON="$tmp/media_adaptation.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MULTI_WINDOW_TRACKING_REPORT_JSON="$tmp/multi_window_tracking.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_NETWORK_FALLBACK_REPORT_JSON="$tmp/network_fallback.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CRASH_RESTART_RECOVERY_REPORT_JSON="$tmp/crash_restart_recovery.json" \
    "$0" --check --out-dir "$tmp/missing-observed-device-pairs" >/dev/null 2>&1; then
    echo "self-test accepted missing observed cross-device pairs" >&2
    exit 1
  fi
  write_synthetic_report "$tmp/cross_device_smoke.json" cross_device_smoke

  python3 - "$tmp/cross_device_smoke.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["topology"]["distinct_device_uras_observed"] = False
report["topology"]["local_provider_boundary_only"] = True
report["coverage"]["distinct_device_uras_observed"] = False
report["coverage"]["local_provider_boundary_only"] = True
path.write_text(json.dumps(report) + "\n", encoding="utf-8")
PY
  if env \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_FRONTEND_PRODUCT_FLOW_REPORT_JSON="$tmp/frontend_product_flow.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_BROWSER_LIFECYCLE_REPORT_JSON="$tmp/browser_lifecycle.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_SMOKE_REPORT_JSON="$tmp/cross_device_smoke.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_PLATFORM_CAPTURE_REPORT_JSON="$tmp/cross_platform_capture.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_INPUT_INJECTION_REPORT_JSON="$tmp/input_injection.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MEDIA_ADAPTATION_REPORT_JSON="$tmp/media_adaptation.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MULTI_WINDOW_TRACKING_REPORT_JSON="$tmp/multi_window_tracking.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_NETWORK_FALLBACK_REPORT_JSON="$tmp/network_fallback.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CRASH_RESTART_RECOVERY_REPORT_JSON="$tmp/crash_restart_recovery.json" \
    "$0" --check --out-dir "$tmp/local-provider-only" >/dev/null 2>&1; then
    echo "self-test accepted local-provider-only cross-device evidence" >&2
    exit 1
  fi
  write_synthetic_report "$tmp/cross_device_smoke.json" cross_device_smoke

  python3 - "$tmp/cross_device_remoteapp.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["scenarios"] = [
    scenario for scenario in report["scenarios"]
    if scenario.get("target_kind") != "application"
]
path.write_text(json.dumps(report) + "\n", encoding="utf-8")
PY
  if env \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_FRONTEND_PRODUCT_FLOW_REPORT_JSON="$tmp/frontend_product_flow.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_BROWSER_LIFECYCLE_REPORT_JSON="$tmp/browser_lifecycle.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_SMOKE_REPORT_JSON="$tmp/cross_device_smoke.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_PLATFORM_CAPTURE_REPORT_JSON="$tmp/cross_platform_capture.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_INPUT_INJECTION_REPORT_JSON="$tmp/input_injection.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MEDIA_ADAPTATION_REPORT_JSON="$tmp/media_adaptation.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MULTI_WINDOW_TRACKING_REPORT_JSON="$tmp/multi_window_tracking.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_NETWORK_FALLBACK_REPORT_JSON="$tmp/network_fallback.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CRASH_RESTART_RECOVERY_REPORT_JSON="$tmp/crash_restart_recovery.json" \
    "$0" --check --out-dir "$tmp/missing-cross-device-remoteapp-target" >/dev/null 2>&1; then
    echo "self-test accepted cross-device RemoteApp report without application target" >&2
    exit 1
  fi
  write_synthetic_report "$tmp/cross_device_remoteapp.json" cross_device_remoteapp

  python3 - "$tmp/cross_platform_capture.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
for platform in report["platforms"]:
    if platform["platform"] == "linux":
        platform["passed_targets"] = ["display"]
        platform["unsupported_targets"] = ["application", "window"]
path.write_text(json.dumps(report) + "\n", encoding="utf-8")
PY
  if env \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_FRONTEND_PRODUCT_FLOW_REPORT_JSON="$tmp/frontend_product_flow.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_BROWSER_LIFECYCLE_REPORT_JSON="$tmp/browser_lifecycle.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_SMOKE_REPORT_JSON="$tmp/cross_device_smoke.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_PLATFORM_CAPTURE_REPORT_JSON="$tmp/cross_platform_capture.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_INPUT_INJECTION_REPORT_JSON="$tmp/input_injection.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MEDIA_ADAPTATION_REPORT_JSON="$tmp/media_adaptation.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MULTI_WINDOW_TRACKING_REPORT_JSON="$tmp/multi_window_tracking.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_NETWORK_FALLBACK_REPORT_JSON="$tmp/network_fallback.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CRASH_RESTART_RECOVERY_REPORT_JSON="$tmp/crash_restart_recovery.json" \
    "$0" --check --out-dir "$tmp/unsupported-cross-platform-capture" >/dev/null 2>&1; then
    echo "self-test accepted unsupported cross-platform capture as product completion" >&2
    exit 1
  fi
  write_synthetic_report "$tmp/cross_platform_capture.json" cross_platform_capture

  python3 - "$tmp/input_injection.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
for platform in report["platforms"]:
    if platform["platform"] == "windows":
        platform["status"] = "unsupported"
path.write_text(json.dumps(report) + "\n", encoding="utf-8")
PY
  if env \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_FRONTEND_PRODUCT_FLOW_REPORT_JSON="$tmp/frontend_product_flow.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_BROWSER_LIFECYCLE_REPORT_JSON="$tmp/browser_lifecycle.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_SMOKE_REPORT_JSON="$tmp/cross_device_smoke.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_PLATFORM_CAPTURE_REPORT_JSON="$tmp/cross_platform_capture.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_INPUT_INJECTION_REPORT_JSON="$tmp/input_injection.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MEDIA_ADAPTATION_REPORT_JSON="$tmp/media_adaptation.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MULTI_WINDOW_TRACKING_REPORT_JSON="$tmp/multi_window_tracking.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_NETWORK_FALLBACK_REPORT_JSON="$tmp/network_fallback.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CRASH_RESTART_RECOVERY_REPORT_JSON="$tmp/crash_restart_recovery.json" \
    "$0" --check --out-dir "$tmp/unsupported-input-injection" >/dev/null 2>&1; then
    echo "self-test accepted unsupported input injection as product completion" >&2
    exit 1
  fi
  write_synthetic_report "$tmp/input_injection.json" input_injection

  python3 - "$tmp/media_adaptation.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["coverage"]["degraded_network"] = False
path.write_text(json.dumps(report) + "\n", encoding="utf-8")
PY
  if env \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_FRONTEND_PRODUCT_FLOW_REPORT_JSON="$tmp/frontend_product_flow.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_BROWSER_LIFECYCLE_REPORT_JSON="$tmp/browser_lifecycle.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_SMOKE_REPORT_JSON="$tmp/cross_device_smoke.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_PLATFORM_CAPTURE_REPORT_JSON="$tmp/cross_platform_capture.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_INPUT_INJECTION_REPORT_JSON="$tmp/input_injection.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MEDIA_ADAPTATION_REPORT_JSON="$tmp/media_adaptation.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MULTI_WINDOW_TRACKING_REPORT_JSON="$tmp/multi_window_tracking.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_NETWORK_FALLBACK_REPORT_JSON="$tmp/network_fallback.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CRASH_RESTART_RECOVERY_REPORT_JSON="$tmp/crash_restart_recovery.json" \
    "$0" --check --out-dir "$tmp/missing-media-coverage" >/dev/null 2>&1; then
    echo "self-test accepted missing media adaptation coverage" >&2
    exit 1
  fi
  write_synthetic_report "$tmp/media_adaptation.json" media_adaptation

  python3 - "$tmp/input_injection.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["product_complete_claim"] = True
path.write_text(json.dumps(report) + "\n", encoding="utf-8")
PY
  if env \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_FRONTEND_PRODUCT_FLOW_REPORT_JSON="$tmp/frontend_product_flow.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_BROWSER_LIFECYCLE_REPORT_JSON="$tmp/browser_lifecycle.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_SMOKE_REPORT_JSON="$tmp/cross_device_smoke.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_PLATFORM_CAPTURE_REPORT_JSON="$tmp/cross_platform_capture.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_INPUT_INJECTION_REPORT_JSON="$tmp/input_injection.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MEDIA_ADAPTATION_REPORT_JSON="$tmp/media_adaptation.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MULTI_WINDOW_TRACKING_REPORT_JSON="$tmp/multi_window_tracking.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_NETWORK_FALLBACK_REPORT_JSON="$tmp/network_fallback.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CRASH_RESTART_RECOVERY_REPORT_JSON="$tmp/crash_restart_recovery.json" \
    "$0" --check --out-dir "$tmp/child-claim" >/dev/null 2>&1; then
    echo "self-test accepted child product_complete_claim" >&2
    exit 1
  fi

  write_synthetic_report "$tmp/input_injection.json" input_injection
  python3 - "$tmp/network_fallback.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["script"] = "tools/scripts/wrong-network-fallback-e2e.sh"
path.write_text(json.dumps(report) + "\n", encoding="utf-8")
PY
  if env \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_FRONTEND_PRODUCT_FLOW_REPORT_JSON="$tmp/frontend_product_flow.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_BROWSER_LIFECYCLE_REPORT_JSON="$tmp/browser_lifecycle.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_SMOKE_REPORT_JSON="$tmp/cross_device_smoke.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_PLATFORM_CAPTURE_REPORT_JSON="$tmp/cross_platform_capture.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_INPUT_INJECTION_REPORT_JSON="$tmp/input_injection.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MEDIA_ADAPTATION_REPORT_JSON="$tmp/media_adaptation.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MULTI_WINDOW_TRACKING_REPORT_JSON="$tmp/multi_window_tracking.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_NETWORK_FALLBACK_REPORT_JSON="$tmp/network_fallback.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CRASH_RESTART_RECOVERY_REPORT_JSON="$tmp/crash_restart_recovery.json" \
    "$0" --check --out-dir "$tmp/wrong-script" >/dev/null 2>&1; then
    echo "self-test accepted wrong report script identity" >&2
    exit 1
  fi
  write_synthetic_report "$tmp/network_fallback.json" network_fallback

  python3 - "$tmp/network_fallback.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
del report["scenarios"]
path.write_text(json.dumps(report) + "\n", encoding="utf-8")
PY
  if env \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_FRONTEND_PRODUCT_FLOW_REPORT_JSON="$tmp/frontend_product_flow.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_BROWSER_LIFECYCLE_REPORT_JSON="$tmp/browser_lifecycle.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_SMOKE_REPORT_JSON="$tmp/cross_device_smoke.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_PLATFORM_CAPTURE_REPORT_JSON="$tmp/cross_platform_capture.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_INPUT_INJECTION_REPORT_JSON="$tmp/input_injection.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MEDIA_ADAPTATION_REPORT_JSON="$tmp/media_adaptation.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MULTI_WINDOW_TRACKING_REPORT_JSON="$tmp/multi_window_tracking.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_NETWORK_FALLBACK_REPORT_JSON="$tmp/network_fallback.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CRASH_RESTART_RECOVERY_REPORT_JSON="$tmp/crash_restart_recovery.json" \
    "$0" --check --out-dir "$tmp/missing-network-route-scenarios" >/dev/null 2>&1; then
    echo "self-test accepted network fallback report without route scenarios" >&2
    exit 1
  fi
  write_synthetic_report "$tmp/network_fallback.json" network_fallback

  if env \
    -u EASYNET_REMOTEAPP_PRODUCT_COMPLETION_BROWSER_LIFECYCLE_REPORT_JSON \
    -u EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_SMOKE_REPORT_JSON \
    -u EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_REMOTEAPP_REPORT_JSON \
    -u EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_PLATFORM_CAPTURE_REPORT_JSON \
    -u EASYNET_REMOTEAPP_PRODUCT_COMPLETION_INPUT_INJECTION_REPORT_JSON \
    -u EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MEDIA_ADAPTATION_REPORT_JSON \
    -u EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MULTI_WINDOW_TRACKING_REPORT_JSON \
    -u EASYNET_REMOTEAPP_PRODUCT_COMPLETION_NETWORK_FALLBACK_REPORT_JSON \
    -u EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_TIMEOUT_WINDOW_REPORT_JSON \
    -u EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_TIMEOUT_APPLICATION_REPORT_JSON \
    -u EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_CANCEL_WINDOW_REPORT_JSON \
    -u EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_CANCEL_APPLICATION_REPORT_JSON \
    -u EASYNET_REMOTEAPP_PRODUCT_COMPLETION_PERMISSION_REVOKE_WINDOW_REPORT_JSON \
    -u EASYNET_REMOTEAPP_PRODUCT_COMPLETION_PERMISSION_REVOKE_APPLICATION_REPORT_JSON \
    -u EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_RESUME_WINDOW_REPORT_JSON \
    -u EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_RESUME_APPLICATION_REPORT_JSON \
    -u EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CRASH_RESTART_RECOVERY_REPORT_JSON \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_FRONTEND_PRODUCT_FLOW_REPORT_JSON="$tmp/frontend_product_flow.json" \
    "$0" --check --out-dir "$tmp/missing-env" >/dev/null 2>&1; then
    echo "self-test accepted missing required report envs" >&2
    exit 1
  fi

  echo "remoteapp-product-completion-e2e self-test ok"
}

case "$MODE" in
  check)
    write_completion_report check
    echo "[remoteapp-product-completion-e2e] PASS: $OUT_DIR/report.md"
    ;;
  self-test)
    bash -n "$0"
    grep -q 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_PLATFORM_CAPTURE_REPORT_JSON' "$0"
    grep -q 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_INPUT_INJECTION_REPORT_JSON' "$0"
    grep -q 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MEDIA_ADAPTATION_REPORT_JSON' "$0"
    grep -q 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MULTI_WINDOW_TRACKING_REPORT_JSON' "$0"
    grep -q 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_NETWORK_FALLBACK_REPORT_JSON' "$0"
    grep -q 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_SMOKE_REPORT_JSON' "$0"
    grep -q 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_REMOTEAPP_REPORT_JSON' "$0"
    grep -q 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_TIMEOUT_WINDOW_REPORT_JSON' "$0"
    grep -q 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_TIMEOUT_APPLICATION_REPORT_JSON' "$0"
    grep -q 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_CANCEL_WINDOW_REPORT_JSON' "$0"
    grep -q 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_CANCEL_APPLICATION_REPORT_JSON' "$0"
    grep -q 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_PERMISSION_REVOKE_WINDOW_REPORT_JSON' "$0"
    grep -q 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_PERMISSION_REVOKE_APPLICATION_REPORT_JSON' "$0"
    grep -q 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_RESUME_WINDOW_REPORT_JSON' "$0"
    grep -q 'EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_RESUME_APPLICATION_REPORT_JSON' "$0"
    grep -q 'topology.local_provider_boundary_only is not false' "$0"
    grep -q 'requires_evidence_json' "$0"
    grep -q 'requires_platforms_passed' "$0"
    grep -q 'requires_network_route_scenarios' "$0"
    grep -q 'requires_cross_device_remoteapp_scenarios' "$0"
    grep -q 'network fallback scenarios summary must be a non-empty list' "$0"
    grep -q 'cross-device RemoteApp scenarios summary must be a non-empty list' "$0"
    grep -q 'unsupported_targets must be empty' "$0"
    grep -q "expected 'passed'" "$0"
    grep -q 'expected_target_kind' "$0"
    grep -q 'target_kind is' "$0"
    grep -q 'host-decoded-frame-window' "$0"
    grep -q 'host-decoded-frame-application' "$0"
    grep -q 'host-view-only-input-window' "$0"
    grep -q 'host-view-only-input-application' "$0"
    grep -q 'product_flow_step_artifacts' "$0"
    grep -q 'product-flow subreport' "$0"
    grep -q 'script is' "$0"
    grep -q 'target_kind is' "$0"
    grep -q 'product-flow step result_json path does not exist' "$0"
    grep -q 'product-flow subreport evidence_json path does not exist' "$0"
    grep -q 'evidence_json path does not exist' "$0"
    grep -q "evidence_json status is" "$0"
    grep -q 'required product-flow step' "$0"
    grep -q 'topology.observed_device_pairs must not be empty' "$0"
    grep -q 'report script is' "$0"
    grep -q 'self-test accepted wrong report script identity' "$0"
    grep -q 'self-test accepted missing evidence_json artifact' "$0"
    grep -q 'self-test accepted wrong lifecycle target_kind' "$0"
    grep -q 'self-test accepted missing frontend product-flow step' "$0"
    grep -q 'self-test accepted product-flow target_kind other than both' "$0"
    grep -q 'self-test accepted missing product-flow step result artifact' "$0"
    grep -q 'self-test accepted missing product-flow subreport evidence artifact' "$0"
    grep -q 'self-test accepted failed product-flow subreport evidence status' "$0"
    grep -q 'self-test accepted failed evidence_json status' "$0"
    grep -q 'self-test accepted wrong product-flow host subreport script identity' "$0"
    grep -q 'self-test accepted wrong product-flow host subreport target_kind' "$0"
    grep -q 'self-test accepted missing observed cross-device pairs' "$0"
    grep -q 'self-test accepted unsupported cross-platform capture as product completion' "$0"
    grep -q 'self-test accepted unsupported input injection as product completion' "$0"
    grep -q 'self-test accepted network fallback report without route scenarios' "$0"
    grep -q 'child verifier must not claim product completion' "$0"
    run_self_test
    ;;
esac
