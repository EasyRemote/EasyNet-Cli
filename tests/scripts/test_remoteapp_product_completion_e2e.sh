#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/tools/scripts/remoteapp-product-completion-e2e.sh"

fail() {
  printf 'test_remoteapp_product_completion_e2e: %s\n' "$1" >&2
  exit 1
}

"$SCRIPT" --self-test >/dev/null

grep -q "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_FRONTEND_PRODUCT_FLOW_REPORT_JSON" "$SCRIPT" || \
  fail "completion gate must require frontend product-flow report"
grep -q "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_BROWSER_LIFECYCLE_REPORT_JSON" "$SCRIPT" || \
  fail "completion gate must require Browser/Tauri lifecycle report"
grep -q "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_BROWSER_TRANSPORT_RESUME_REPORT_JSON" "$SCRIPT" || \
  fail "completion gate must require real browser transport-resume report"
grep -q "requires_transport_resume_summary" "$SCRIPT" || \
  fail "completion gate must require transport-resume summaries"
grep -q "real_browser_transport_resume" "$SCRIPT" || \
  fail "completion gate must reject lease survival as transport resume"
grep -q "self-test accepted browser transport resume without a new PeerConnection" "$SCRIPT" || \
  fail "completion gate self-test must reject fake PeerConnection resume"
grep -q "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_SMOKE_REPORT_JSON" "$SCRIPT" || \
  fail "completion gate must require cross-device smoke report"
grep -q "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_REMOTEAPP_REPORT_JSON" "$SCRIPT" || \
  fail "completion gate must require cross-device RemoteApp product report"
grep -q "tools/scripts/remoteapp-cross-device-remoteapp-e2e.sh" "$SCRIPT" || \
  fail "completion gate must pin cross-device RemoteApp verifier identity"
grep -q "requires_cross_device_remoteapp_scenarios" "$SCRIPT" || \
  fail "completion gate must require cross-device RemoteApp target scenarios"
grep -q "self-test accepted cross-device RemoteApp report without application target" "$SCRIPT" || \
  fail "completion gate self-test must cover incomplete cross-device RemoteApp targets"
grep -q "cross-device RemoteApp target .* remoteapp_summary must be an object" "$SCRIPT" || \
  fail "completion gate must reject cross-device RemoteApp reports without per-target summaries"
grep -q "self-test accepted cross-device RemoteApp report without summaries" "$SCRIPT" || \
  fail "completion gate self-test must cover missing cross-device RemoteApp summaries"
grep -q '"production_signaling_bound"' "$SCRIPT" || \
  fail "completion gate must require the production cross-device signaling path"
grep -q '"diagnostic_attach_absent"' "$SCRIPT" || \
  fail "completion gate must reject diagnostic attach in cross-device product evidence"
grep -q "remoteapp_summary.selected_candidate_pair_id must be recorded" "$SCRIPT" || \
  fail "completion gate must require a selected cross-device WebRTC pair"
grep -q "self-test accepted cross-device RemoteApp report without a selected WebRTC pair" "$SCRIPT" || \
  fail "completion gate self-test must cover missing selected cross-device pair evidence"
grep -q "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_PLATFORM_CAPTURE_REPORT_JSON" "$SCRIPT" || \
  fail "completion gate must require cross-platform capture report"
grep -q "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_INPUT_INJECTION_REPORT_JSON" "$SCRIPT" || \
  fail "completion gate must require input injection report"
grep -q "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MEDIA_ADAPTATION_REPORT_JSON" "$SCRIPT" || \
  fail "completion gate must require media adaptation report"
grep -q "requires_media_scenarios" "$SCRIPT" || \
  fail "completion gate must require media adaptation scenario summaries"
grep -q "media adaptation scenarios summary must be a non-empty list" "$SCRIPT" || \
  fail "completion gate must reject media reports without scenario summaries"
grep -q "self-test accepted media adaptation report without scenarios" "$SCRIPT" || \
  fail "completion gate self-test must cover missing media scenario summaries"
grep -q "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MULTI_WINDOW_TRACKING_REPORT_JSON" "$SCRIPT" || \
  fail "completion gate must require multi-window tracking report"
grep -q "requires_multi_window_scenarios" "$SCRIPT" || \
  fail "completion gate must require multi-window scenario summaries"
grep -q "multi-window tracking scenarios summary must be a non-empty list" "$SCRIPT" || \
  fail "completion gate must reject multi-window reports without scenario summaries"
grep -q "self-test accepted unsupported multi-display application as product completion" "$SCRIPT" || \
  fail "completion gate self-test must reject unsupported multi-display app product completion"
grep -q "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_NETWORK_FALLBACK_REPORT_JSON" "$SCRIPT" || \
  fail "completion gate must require network fallback report"
grep -q "requires_network_route_scenarios" "$SCRIPT" || \
  fail "completion gate must require route-scenario summaries for network fallback"
grep -q "network fallback scenarios summary must be a non-empty list" "$SCRIPT" || \
  fail "completion gate must reject network fallback reports without route scenarios"
grep -q "self-test accepted network fallback report without route scenarios" "$SCRIPT" || \
  fail "completion gate self-test must cover missing network route scenarios"
grep -q "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_TIMEOUT_WINDOW_REPORT_JSON" "$SCRIPT" || \
  fail "completion gate must require window session timeout report"
grep -q "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_TIMEOUT_APPLICATION_REPORT_JSON" "$SCRIPT" || \
  fail "completion gate must require application session timeout report"
grep -q "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_CANCEL_WINDOW_REPORT_JSON" "$SCRIPT" || \
  fail "completion gate must require window session cancel report"
grep -q "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_CANCEL_APPLICATION_REPORT_JSON" "$SCRIPT" || \
  fail "completion gate must require application session cancel report"
grep -q "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_PERMISSION_REVOKE_WINDOW_REPORT_JSON" "$SCRIPT" || \
  fail "completion gate must require window permission revoke report"
grep -q "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_PERMISSION_REVOKE_APPLICATION_REPORT_JSON" "$SCRIPT" || \
  fail "completion gate must require application permission revoke report"
grep -q "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_RESUME_WINDOW_REPORT_JSON" "$SCRIPT" || \
  fail "completion gate must require window session resume report"
grep -q "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_SESSION_RESUME_APPLICATION_REPORT_JSON" "$SCRIPT" || \
  fail "completion gate must require application session resume report"
grep -q "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CRASH_RESTART_RECOVERY_REPORT_JSON" "$SCRIPT" || \
  fail "completion gate must require crash/restart recovery report"
grep -q "requires_crash_restart_recovery_scenarios" "$SCRIPT" || \
  fail "completion gate must require crash/restart recovery scenario summaries"
grep -q "crash/restart recovery scenarios summary must be a non-empty list" "$SCRIPT" || \
  fail "completion gate must reject crash/restart reports without scenario summaries"
grep -q "self-test accepted crash/restart recovery report without scenarios" "$SCRIPT" || \
  fail "completion gate self-test must cover missing crash/restart scenario summaries"
grep -q "local-provider-only cross-device evidence" "$SCRIPT" || \
  fail "completion gate self-test must reject local-provider-only evidence"
grep -q "child verifier must not claim product completion" "$SCRIPT" || \
  fail "completion gate must reject child product-complete claims"
grep -q "requires_evidence_json" "$SCRIPT" || \
  fail "completion gate must require live evidence_json artifacts"
grep -q "requires_frontend_flow_summary" "$SCRIPT" || \
  fail "completion gate must require frontend flow summaries"
grep -q "frontend_flow_summary must be an object" "$SCRIPT" || \
  fail "completion gate must reject frontend product-flow reports without summaries"
grep -q "self-test accepted frontend product-flow report without summary" "$SCRIPT" || \
  fail "completion gate self-test must cover missing frontend flow summaries"
grep -q "requires_platforms_passed" "$SCRIPT" || \
  fail "completion gate must require product platforms to pass, not just be covered"
grep -q "requires_cross_platform_capture_scenarios" "$SCRIPT" || \
  fail "completion gate must require cross-platform capture scenario summaries"
grep -q "cross-platform capture .* scenarios summary must be a non-empty list" "$SCRIPT" || \
  fail "completion gate must reject capture reports without per-target scenarios"
grep -q "self-test accepted cross-platform capture report without scenarios" "$SCRIPT" || \
  fail "completion gate self-test must cover missing cross-platform capture scenarios"
grep -q "requires_input_injection_scenarios" "$SCRIPT" || \
  fail "completion gate must require input injection summaries"
grep -q "input injection .* input_summary must be an object" "$SCRIPT" || \
  fail "completion gate must reject input reports without per-platform summaries"
grep -q "self-test accepted input injection report without summaries" "$SCRIPT" || \
  fail "completion gate self-test must cover missing input injection summaries"
grep -q "unsupported_targets must be empty" "$SCRIPT" || \
  fail "completion gate must reject unsupported cross-platform capture targets"
grep -q "expected 'passed'" "$SCRIPT" || \
  fail "completion gate must reject unsupported input platforms"
grep -q "expected_target_kind" "$SCRIPT" || \
  fail "completion gate must require full product-flow target coverage"
grep -q "target_kind is" "$SCRIPT" || \
  fail "completion gate must reject product-flow reports that are not target_kind=both"
grep -q "host-decoded-frame-window" "$SCRIPT" || \
  fail "completion gate must require window decoded-frame product-flow evidence"
grep -q "host-target-picker-freshness-application" "$SCRIPT" || \
  fail "completion gate must require application target-picker freshness evidence"
grep -q "window_target_picker_fresh" "$SCRIPT" || \
  fail "completion gate must require window target-picker freshness summary"
grep -q "application_target_picker_fresh" "$SCRIPT" || \
  fail "completion gate must require application target-picker freshness summary"
grep -q "host-decoded-frame-application" "$SCRIPT" || \
  fail "completion gate must require application decoded-frame product-flow evidence"
grep -q "host-view-only-input-window" "$SCRIPT" || \
  fail "completion gate must require window view-only input product-flow evidence"
grep -q "host-view-only-input-application" "$SCRIPT" || \
  fail "completion gate must require application view-only input product-flow evidence"
grep -q "product_flow_step_artifacts" "$SCRIPT" || \
  fail "completion gate must require product-flow step artifacts"
grep -q "tools/scripts/host-remoteapp-permission-subject-e2e.sh" "$SCRIPT" || \
  fail "completion gate must pin permission-subject product-flow subreport identity"
python3 - "$SCRIPT" <<'PY'
import pathlib
import re
import sys

text = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
usage_start = text.index("Required report environment:")
usage_end = text.index("Required signed-campaign environment", usage_start)
documented = set(
    re.findall(
        r"\bEASYNET_REMOTEAPP_PRODUCT_COMPLETION_[A-Z0-9_]+_REPORT_JSON\b",
        text[usage_start:usage_end],
    )
)
required = set(
    re.findall(
        r'"env":\s*"(EASYNET_REMOTEAPP_PRODUCT_COMPLETION_[A-Z0-9_]+_REPORT_JSON)"',
        text[usage_end:],
    )
)
for prefix in re.findall(
    r'lifecycle_required\(\s*"[^"]+",\s*"(EASYNET_REMOTEAPP_PRODUCT_COMPLETION_[A-Z0-9_]+)"',
    text[usage_end:],
):
    required.add(f"{prefix}_WINDOW_REPORT_JSON")
    required.add(f"{prefix}_APPLICATION_REPORT_JSON")
if documented != required:
    raise SystemExit(
        "product completion usage/report contract drift: "
        f"missing={sorted(required - documented)}, "
        f"obsolete={sorted(documented - required)}"
    )
PY
grep -q "tools/scripts/host-remoteapp-target-picker-freshness-e2e.sh" "$SCRIPT" || \
  fail "completion gate must pin target-picker freshness product-flow subreport identity"
grep -q "tools/scripts/host-remoteapp-decoded-frame-e2e.sh" "$SCRIPT" || \
  fail "completion gate must pin decoded-frame product-flow subreport identity"
grep -q "tools/scripts/host-remoteapp-view-only-input-safety-e2e.sh" "$SCRIPT" || \
  fail "completion gate must pin view-only input product-flow subreport identity"
grep -q "expected_target_kind" "$SCRIPT" || \
  fail "completion gate must pin product-flow host subreport target kinds"
grep -q "product-flow subreport" "$SCRIPT" || \
  fail "completion gate must inspect product-flow subreports"
grep -q "target_kind is" "$SCRIPT" || \
  fail "completion gate must reject wrong product-flow host subreport target kinds"
grep -q "product-flow step result_json path does not exist" "$SCRIPT" || \
  fail "completion gate must reject missing product-flow step result artifacts"
grep -q "product-flow subreport evidence_json path does not exist" "$SCRIPT" || \
  fail "completion gate must reject missing product-flow subreport evidence artifacts"
grep -q "evidence_json path does not exist" "$SCRIPT" || \
  fail "completion gate must reject missing evidence_json artifacts"
grep -q "evidence_json status is" "$SCRIPT" || \
  fail "completion gate must reject failed evidence_json artifacts"
grep -q 'LIVE_EVIDENCE_ORIGIN = "live_runner"' "$SCRIPT" || \
  fail "completion gate must define the live evidence origin"
grep -q 'CONTRACT_SELF_TEST_ORIGIN = "contract_self_test"' "$SCRIPT" || \
  fail "completion gate must define the contract self-test origin"
grep -q "self-test accepted contract_self_test report evidence_origin" "$SCRIPT" || \
  fail "completion gate self-test must reject self-test report provenance"
grep -q "self-test accepted missing evidence_json evidence_origin" "$SCRIPT" || \
  fail "completion gate self-test must reject missing evidence provenance"
grep -q "self-test accepted unknown product-flow step evidence_origin" "$SCRIPT" || \
  fail "completion gate self-test must reject unknown nested step provenance"
grep -q "required product-flow step" "$SCRIPT" || \
  fail "completion gate must reject incomplete frontend product-flow reports"
grep -q "topology.observed_device_pairs must not be empty" "$SCRIPT" || \
  fail "completion gate must reject cross-device reports without observed pairs"
grep -q "self-test accepted missing evidence_json artifact" "$SCRIPT" || \
  fail "completion gate self-test must cover missing evidence_json artifacts"
grep -q "self-test accepted wrong lifecycle target_kind" "$SCRIPT" || \
  fail "completion gate self-test must cover wrong lifecycle target kinds"
grep -q "requires_lifecycle_summary" "$SCRIPT" || \
  fail "completion gate must require lifecycle summary reports"
grep -q "lifecycle_summary must be an object" "$SCRIPT" || \
  fail "completion gate must reject lifecycle reports without summaries"
grep -q "self-test accepted lifecycle report without summary" "$SCRIPT" || \
  fail "completion gate self-test must cover missing lifecycle summaries"
grep -q "self-test accepted missing frontend product-flow step" "$SCRIPT" || \
  fail "completion gate self-test must cover missing product-flow steps"
grep -q "self-test accepted product-flow target_kind other than both" "$SCRIPT" || \
  fail "completion gate self-test must cover product-flow target-kind narrowing"
grep -q "self-test accepted missing product-flow step result artifact" "$SCRIPT" || \
  fail "completion gate self-test must cover missing product-flow step result artifacts"
grep -q "self-test accepted missing product-flow subreport evidence artifact" "$SCRIPT" || \
  fail "completion gate self-test must cover missing product-flow subreport evidence artifacts"
grep -q "self-test accepted failed product-flow subreport evidence status" "$SCRIPT" || \
  fail "completion gate self-test must cover failed product-flow subreport evidence status"
grep -q "self-test accepted failed evidence_json status" "$SCRIPT" || \
  fail "completion gate self-test must cover failed evidence_json status"
grep -q "self-test accepted wrong product-flow host subreport script identity" "$SCRIPT" || \
  fail "completion gate self-test must cover wrong product-flow host subreport script identities"
grep -q "self-test accepted wrong product-flow host subreport target_kind" "$SCRIPT" || \
  fail "completion gate self-test must cover wrong product-flow host subreport target kinds"
grep -q "self-test accepted missing observed cross-device pairs" "$SCRIPT" || \
  fail "completion gate self-test must cover missing observed cross-device pairs"
grep -q "self-test accepted unsupported cross-platform capture as product completion" "$SCRIPT" || \
  fail "completion gate self-test must cover unsupported capture product-completion rejection"
grep -q "self-test accepted unsupported input injection as product completion" "$SCRIPT" || \
  fail "completion gate self-test must cover unsupported input product-completion rejection"
grep -q "expected_script" "$SCRIPT" || \
  fail "completion gate must pin expected report script identities"
grep -q "report script is" "$SCRIPT" || \
  fail "completion gate must reject wrong report script identities"
grep -q "tools/scripts/host-remoteapp-session-timeout-e2e.sh" "$SCRIPT" || \
  fail "completion gate must pin host timeout report identity"
grep -q "tools/scripts/remoteapp-network-fallback-e2e.sh" "$SCRIPT" || \
  fail "completion gate must pin network fallback report identity"
grep -q 'and not contract_fixture_mode' "$SCRIPT" || \
  fail "completion gate must reject contract fixtures from candidate eligibility"
grep -q 'easynet.remoteapp.product-completion-candidate.v1' "$SCRIPT" || \
  fail "completion gate must emit the non-claim candidate schema"
grep -q 'product_complete_claim = False' "$SCRIPT" || \
  fail "completion gate must never mint the final product-complete claim"
grep -q 'completion_signature_pending' "$SCRIPT" || \
  fail "eligible completion candidates must require independent authorization"
grep -q 'contract_fixture and cannot be accepted as live evidence' "$SCRIPT" || \
  fail "completion gate must reject synthetic fixtures in production check mode"
grep -q 'self-test accepted contract_fixture as live product evidence' "$SCRIPT" || \
  fail "completion gate self-test must reject fixture laundering"

OUT_DIR="$(mktemp -d)"
if "$SCRIPT" --check --out-dir "$OUT_DIR" >/tmp/remoteapp-product-completion-missing.out 2>/tmp/remoteapp-product-completion-missing.err; then
  fail "completion gate must fail closed when required report envs are missing"
fi

python3 - "$OUT_DIR/report.json" <<'PY'
import json
import sys

report = json.load(open(sys.argv[1], encoding="utf-8"))
assert report["status"] == "failed"
assert report["evidence_origin"] == "live_runner"
assert report["product_complete_claim"] is False
assert report["product_complete_eligible"] is False
assert report["finalization_state"] == "not_eligible"
assert report["schema"] == "easynet.remoteapp.product-completion-candidate.v1"
assert report["required_evidence_count"] == 19
assert any("missing required report env" in error for error in report["errors"])
PY

echo "test_remoteapp_product_completion_e2e: ok"
