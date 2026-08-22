#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh"
HARNESS="$REPO_ROOT/tools/scripts/frontend-remoteapp-product-flow-e2e.sh"

fail() {
  printf 'test_check_remoteapp_frontend_product_flow_e2e: %s\n' "$1" >&2
  exit 1
}

"$SCRIPT" >/dev/null
"$HARNESS" --self-test >/dev/null
EASYNET_FRONTEND_REMOTEAPP_PRODUCT_E2E_OUT_DIR="$REPO_ROOT/target/test/frontend-remoteapp-product-flow-skip" \
  "$HARNESS" >/dev/null
grep -q '"status": "skipped"' "$REPO_ROOT/target/test/frontend-remoteapp-product-flow-skip/report.json" || \
  fail "harness did not emit skipped report when run gate was absent"

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT
mkdir -p \
  "$SB/tools/scripts" \
  "$SB/docs/design" \
  "$SB/pr/20260822-remoteapp-product-closure" \
  "$SB/Frontend/src/components/easynet"

cp "$SCRIPT" "$SB/tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh"
cp "$HARNESS" "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-permission-subject-e2e.sh" \
  "$SB/tools/scripts/host-remoteapp-permission-subject-e2e.sh"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-target-picker-freshness-e2e.sh" \
  "$SB/tools/scripts/host-remoteapp-target-picker-freshness-e2e.sh"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
  "$SB/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-view-only-input-safety-e2e.sh" \
  "$SB/tools/scripts/host-remoteapp-view-only-input-safety-e2e.sh"
cp "$REPO_ROOT/docs/design/remoteapp-product-readiness-audit-2026-08-22.md" \
  "$SB/docs/design/remoteapp-product-readiness-audit-2026-08-22.md"
cp "$REPO_ROOT/pr/20260822-remoteapp-product-closure/02-evidence-audit.md" \
  "$SB/pr/20260822-remoteapp-product-closure/02-evidence-audit.md"
cp "$REPO_ROOT/../EasyNet/Frontend/src/components/easynet/DeviceMediaAccess.test.tsx" \
  "$SB/Frontend/src/components/easynet/DeviceMediaAccess.test.tsx"
chmod +x "$SB/tools/scripts/"*.sh

CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_ROOT="$SB" \
CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_FRONTEND_ROOT="$SB/Frontend" \
  bash "$SB/tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh" >/dev/null

cp "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh" \
  "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh.good"
perl -0pi -e 's/run_step daemon-readiness-preflight run_daemon_readiness_preflight/# daemon readiness preflight removed/g' \
  "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh"
if CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_ROOT="$SB" \
  CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_FRONTEND_ROOT="$SB/Frontend" \
  bash "$SB/tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh" >/dev/null 2>&1; then
  fail "checker accepted product-flow harness without daemon readiness preflight step"
fi
mv "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh.good" \
  "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh"

cp "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh" \
  "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh.good"
perl -0pi -e 's/hub_api_endpoint=/hub_api_hidden=/g' \
  "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh"
if CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_ROOT="$SB" \
  CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_FRONTEND_ROOT="$SB/Frontend" \
  bash "$SB/tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh" >/dev/null 2>&1; then
  fail "checker accepted product-flow harness without Hub API endpoint diagnostics"
fi
mv "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh.good" \
  "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh"

cp "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh" \
  "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh.good"
perl -0pi -e 's/host-remoteapp-decoded-frame-e2e\.sh/host-remoteapp-untyped-frame-e2e.sh/g' \
  "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh"
if CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_ROOT="$SB" \
  CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_FRONTEND_ROOT="$SB/Frontend" \
  bash "$SB/tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh" >/dev/null 2>&1; then
  fail "checker accepted product-flow harness without decoded-frame E2E"
fi
mv "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh.good" \
  "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh"

cp "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh" \
  "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh.good"
perl -0pi -e 's/--require-screen-capture-granted/--allow-denied-screen-capture/g' \
  "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh"
if CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_ROOT="$SB" \
  CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_FRONTEND_ROOT="$SB/Frontend" \
  bash "$SB/tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh" >/dev/null 2>&1; then
  fail "checker accepted product-flow harness without strict permission preflight"
fi
mv "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh.good" \
  "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh"

cp "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh" \
  "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh.good"
perl -0pi -e 's/write_json_report "failed" "step \$name failed"/echo "step failed without top-level report"/g' \
  "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh"
if CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_ROOT="$SB" \
  CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_FRONTEND_ROOT="$SB/Frontend" \
  bash "$SB/tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh" >/dev/null 2>&1; then
  fail "checker accepted product-flow harness without top-level failed report"
fi
mv "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh.good" \
  "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh"

cp "$SB/Frontend/src/components/easynet/DeviceMediaAccess.test.tsx" \
  "$SB/Frontend/src/components/easynet/DeviceMediaAccess.test.tsx.good"
perl -0pi -e 's/runs the remote desktop UI flow from target picker through session end/runs a local mocked card render/g' \
  "$SB/Frontend/src/components/easynet/DeviceMediaAccess.test.tsx"
if CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_ROOT="$SB" \
  CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_FRONTEND_ROOT="$SB/Frontend" \
  bash "$SB/tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh" >/dev/null 2>&1; then
  fail "checker accepted frontend coverage without picker-to-end UI flow"
fi
mv "$SB/Frontend/src/components/easynet/DeviceMediaAccess.test.tsx.good" \
  "$SB/Frontend/src/components/easynet/DeviceMediaAccess.test.tsx"

cp "$SB/docs/design/remoteapp-product-readiness-audit-2026-08-22.md" \
  "$SB/docs/design/remoteapp-product-readiness-audit-2026-08-22.md.good"
perl -0pi -e 's/RemoteApp interactive desktop product: incomplete/RemoteApp interactive desktop product: complete/g' \
  "$SB/docs/design/remoteapp-product-readiness-audit-2026-08-22.md"
if CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_ROOT="$SB" \
  CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_FRONTEND_ROOT="$SB/Frontend" \
  bash "$SB/tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh" >/dev/null 2>&1; then
  fail "checker accepted product audit claiming completion"
fi
mv "$SB/docs/design/remoteapp-product-readiness-audit-2026-08-22.md.good" \
  "$SB/docs/design/remoteapp-product-readiness-audit-2026-08-22.md"

echo "test_check_remoteapp_frontend_product_flow_e2e: ok"
