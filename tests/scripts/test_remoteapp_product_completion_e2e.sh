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
grep -q "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_DEVICE_SMOKE_REPORT_JSON" "$SCRIPT" || \
  fail "completion gate must require cross-device smoke report"
grep -q "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CROSS_PLATFORM_CAPTURE_REPORT_JSON" "$SCRIPT" || \
  fail "completion gate must require cross-platform capture report"
grep -q "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_INPUT_INJECTION_REPORT_JSON" "$SCRIPT" || \
  fail "completion gate must require input injection report"
grep -q "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MEDIA_ADAPTATION_REPORT_JSON" "$SCRIPT" || \
  fail "completion gate must require media adaptation report"
grep -q "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_MULTI_WINDOW_TRACKING_REPORT_JSON" "$SCRIPT" || \
  fail "completion gate must require multi-window tracking report"
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
grep -q "local-provider-only cross-device evidence" "$SCRIPT" || \
  fail "completion gate self-test must reject local-provider-only evidence"
grep -q "child verifier must not claim product completion" "$SCRIPT" || \
  fail "completion gate must reject child product-complete claims"
grep -q "requires_evidence_json" "$SCRIPT" || \
  fail "completion gate must require live evidence_json artifacts"
grep -q "requires_platforms_passed" "$SCRIPT" || \
  fail "completion gate must require product platforms to pass, not just be covered"
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
grep -q "required product-flow step" "$SCRIPT" || \
  fail "completion gate must reject incomplete frontend product-flow reports"
grep -q "topology.observed_device_pairs must not be empty" "$SCRIPT" || \
  fail "completion gate must reject cross-device reports without observed pairs"
grep -q "self-test accepted missing evidence_json artifact" "$SCRIPT" || \
  fail "completion gate self-test must cover missing evidence_json artifacts"
grep -q "self-test accepted wrong lifecycle target_kind" "$SCRIPT" || \
  fail "completion gate self-test must cover wrong lifecycle target kinds"
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
grep -q '"product_complete_claim": effective_status == "passed"' "$SCRIPT" || \
  fail "completion gate must be the single aggregate product completion claim"

OUT_DIR="$(mktemp -d)"
if "$SCRIPT" --check --out-dir "$OUT_DIR" >/tmp/remoteapp-product-completion-missing.out 2>/tmp/remoteapp-product-completion-missing.err; then
  fail "completion gate must fail closed when required report envs are missing"
fi

python3 - "$OUT_DIR/report.json" <<'PY'
import json
import sys

report = json.load(open(sys.argv[1], encoding="utf-8"))
assert report["status"] == "failed"
assert report["product_complete_claim"] is False
assert report["required_evidence_count"] == 17
assert any("missing required report env" in error for error in report["errors"])
PY

echo "test_remoteapp_product_completion_e2e: ok"
