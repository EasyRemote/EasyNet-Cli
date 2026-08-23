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
  EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_PLATFORM_CAPTURE_REPORT_JSON
  EASYNET_REMOTEAPP_PRODUCT_COMPLETION_INPUT_INJECTION_REPORT_JSON
  EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MEDIA_ADAPTATION_REPORT_JSON
  EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MULTI_WINDOW_TRACKING_REPORT_JSON
  EASYNET_REMOTEAPP_PRODUCT_COMPLETION_NETWORK_FALLBACK_REPORT_JSON
  EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_TIMEOUT_REPORT_JSON
  EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_CANCEL_REPORT_JSON
  EASYNET_REMOTEAPP_PRODUCT_COMPLETION_PERMISSION_REVOKE_REPORT_JSON
  EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_RESUME_REPORT_JSON
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

required = [
    {
        "id": "frontend_product_flow",
        "env": "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_FRONTEND_PRODUCT_FLOW_REPORT_JSON",
        "expected_script": "tools/scripts/frontend-remoteapp-product-flow-e2e.sh",
        "coverage_keys": [],
        "evidence_contract_contains": [
            "Browser/Tauri RemoteApp lifecycle evidence",
            "cross-device product smoke with distinct device URAs",
        ],
    },
    {
        "id": "browser_lifecycle",
        "env": "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_BROWSER_LIFECYCLE_REPORT_JSON",
        "expected_script": "tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh",
        "coverage_keys": [],
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
    },
    {
        "id": "cross_platform_capture",
        "env": "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_PLATFORM_CAPTURE_REPORT_JSON",
        "expected_script": "tools/scripts/remoteapp-cross-platform-capture-e2e.sh",
        "coverage_keys": ["macos", "windows", "linux"],
    },
    {
        "id": "input_injection",
        "env": "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_INPUT_INJECTION_REPORT_JSON",
        "expected_script": "tools/scripts/remoteapp-input-injection-e2e.sh",
        "coverage_keys": ["macos", "windows", "linux"],
    },
    {
        "id": "media_adaptation",
        "env": "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MEDIA_ADAPTATION_REPORT_JSON",
        "expected_script": "tools/scripts/remoteapp-media-adaptation-e2e.sh",
        "coverage_keys": ["baseline", "degraded_network", "backpressure"],
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
    },
    {
        "id": "network_fallback",
        "env": "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_NETWORK_FALLBACK_REPORT_JSON",
        "expected_script": "tools/scripts/remoteapp-network-fallback-e2e.sh",
        "coverage_keys": ["direct", "stun_srflx", "turn_relay", "easynet_relay"],
    },
    {
        "id": "session_timeout",
        "env": "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_TIMEOUT_REPORT_JSON",
        "expected_script": "tools/scripts/host-remoteapp-session-timeout-e2e.sh",
        "coverage_keys": [],
    },
    {
        "id": "session_cancel",
        "env": "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_CANCEL_REPORT_JSON",
        "expected_script": "tools/scripts/host-remoteapp-session-cancel-e2e.sh",
        "coverage_keys": [],
    },
    {
        "id": "permission_revoke",
        "env": "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_PERMISSION_REVOKE_REPORT_JSON",
        "expected_script": "tools/scripts/host-remoteapp-permission-revoke-e2e.sh",
        "coverage_keys": [],
    },
    {
        "id": "session_resume",
        "env": "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_RESUME_REPORT_JSON",
        "expected_script": "tools/scripts/host-remoteapp-session-resume-e2e.sh",
        "coverage_keys": [],
    },
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
    },
]

checks = []
errors = []

def add_error(item_id, message):
    errors.append(f"{item_id}: {message}")

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
    "cross_platform_capture": "tools/scripts/remoteapp-cross-platform-capture-e2e.sh",
    "input_injection": "tools/scripts/remoteapp-input-injection-e2e.sh",
    "media_adaptation": "tools/scripts/remoteapp-media-adaptation-e2e.sh",
    "multi_window_tracking": "tools/scripts/remoteapp-multi-window-tracking-e2e.sh",
    "network_fallback": "tools/scripts/remoteapp-network-fallback-e2e.sh",
    "session_timeout": "tools/scripts/host-remoteapp-session-timeout-e2e.sh",
    "session_cancel": "tools/scripts/host-remoteapp-session-cancel-e2e.sh",
    "permission_revoke": "tools/scripts/host-remoteapp-permission-revoke-e2e.sh",
    "session_resume": "tools/scripts/host-remoteapp-session-resume-e2e.sh",
    "crash_restart_recovery": "tools/scripts/remoteapp-crash-restart-recovery-e2e.sh",
}
report = {
    "script": script_by_id[item_id],
    "status": "passed",
    "product_complete_claim": False,
    "coverage": coverage_by_id.get(item_id, {}),
}
if item_id == "frontend_product_flow":
    report["evidence_contract"] = [
        "Browser/Tauri RemoteApp lifecycle evidence",
        "cross-device product smoke with distinct device URAs",
    ]
if item_id == "cross_device_smoke":
    report["topology"] = {
        "requires_distinct_devices": True,
        "distinct_device_uras_observed": True,
        "local_provider_boundary_only": False,
    }
path.parent.mkdir(parents=True, exist_ok=True)
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
    cross_platform_capture
    input_injection
    media_adaptation
    multi_window_tracking
    network_fallback
    session_timeout
    session_cancel
    permission_revoke
    session_resume
    crash_restart_recovery
  )
  for id in "${ids[@]}"; do
    write_synthetic_report "$tmp/$id.json" "$id"
  done

  env \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_FRONTEND_PRODUCT_FLOW_REPORT_JSON="$tmp/frontend_product_flow.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_BROWSER_LIFECYCLE_REPORT_JSON="$tmp/browser_lifecycle.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_SMOKE_REPORT_JSON="$tmp/cross_device_smoke.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_PLATFORM_CAPTURE_REPORT_JSON="$tmp/cross_platform_capture.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_INPUT_INJECTION_REPORT_JSON="$tmp/input_injection.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MEDIA_ADAPTATION_REPORT_JSON="$tmp/media_adaptation.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MULTI_WINDOW_TRACKING_REPORT_JSON="$tmp/multi_window_tracking.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_NETWORK_FALLBACK_REPORT_JSON="$tmp/network_fallback.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_TIMEOUT_REPORT_JSON="$tmp/session_timeout.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_CANCEL_REPORT_JSON="$tmp/session_cancel.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_PERMISSION_REVOKE_REPORT_JSON="$tmp/permission_revoke.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_RESUME_REPORT_JSON="$tmp/session_resume.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CRASH_RESTART_RECOVERY_REPORT_JSON="$tmp/crash_restart_recovery.json" \
    "$0" --check --out-dir "$tmp/pass" >/dev/null
  grep -q '"product_complete_claim": true' "$tmp/pass/report.json"

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
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_TIMEOUT_REPORT_JSON="$tmp/session_timeout.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_CANCEL_REPORT_JSON="$tmp/session_cancel.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_PERMISSION_REVOKE_REPORT_JSON="$tmp/permission_revoke.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_RESUME_REPORT_JSON="$tmp/session_resume.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CRASH_RESTART_RECOVERY_REPORT_JSON="$tmp/crash_restart_recovery.json" \
    "$0" --check --out-dir "$tmp/local-provider-only" >/dev/null 2>&1; then
    echo "self-test accepted local-provider-only cross-device evidence" >&2
    exit 1
  fi
  write_synthetic_report "$tmp/cross_device_smoke.json" cross_device_smoke

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
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_TIMEOUT_REPORT_JSON="$tmp/session_timeout.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_CANCEL_REPORT_JSON="$tmp/session_cancel.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_PERMISSION_REVOKE_REPORT_JSON="$tmp/permission_revoke.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_RESUME_REPORT_JSON="$tmp/session_resume.json" \
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
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_TIMEOUT_REPORT_JSON="$tmp/session_timeout.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_CANCEL_REPORT_JSON="$tmp/session_cancel.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_PERMISSION_REVOKE_REPORT_JSON="$tmp/permission_revoke.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_RESUME_REPORT_JSON="$tmp/session_resume.json" \
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
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_TIMEOUT_REPORT_JSON="$tmp/session_timeout.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_CANCEL_REPORT_JSON="$tmp/session_cancel.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_PERMISSION_REVOKE_REPORT_JSON="$tmp/permission_revoke.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_RESUME_REPORT_JSON="$tmp/session_resume.json" \
    EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CRASH_RESTART_RECOVERY_REPORT_JSON="$tmp/crash_restart_recovery.json" \
    "$0" --check --out-dir "$tmp/wrong-script" >/dev/null 2>&1; then
    echo "self-test accepted wrong report script identity" >&2
    exit 1
  fi
  write_synthetic_report "$tmp/network_fallback.json" network_fallback

  if env \
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
    grep -q 'topology.local_provider_boundary_only is not false' "$0"
    grep -q 'report script is' "$0"
    grep -q 'self-test accepted wrong report script identity' "$0"
    grep -q 'child verifier must not claim product completion' "$0"
    run_self_test
    ;;
esac
