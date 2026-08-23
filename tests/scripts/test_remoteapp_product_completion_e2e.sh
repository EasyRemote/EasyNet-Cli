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
grep -q "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_PERMISSION_REVOKE_REPORT_JSON" "$SCRIPT" || \
  fail "completion gate must require permission revoke report"
grep -q "EASYNET_REMOTEAPP_PRODUCT_COMPLETION_CRASH_RESTART_RECOVERY_REPORT_JSON" "$SCRIPT" || \
  fail "completion gate must require crash/restart recovery report"
grep -q "local-provider-only cross-device evidence" "$SCRIPT" || \
  fail "completion gate self-test must reject local-provider-only evidence"
grep -q "child verifier must not claim product completion" "$SCRIPT" || \
  fail "completion gate must reject child product-complete claims"
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
assert report["required_evidence_count"] == 13
assert any("missing required report env" in error for error in report["errors"])
PY

echo "test_remoteapp_product_completion_e2e: ok"
