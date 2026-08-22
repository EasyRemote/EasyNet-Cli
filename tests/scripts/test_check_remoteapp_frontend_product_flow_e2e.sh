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
  "$SB/plugins/remote-desktop/src" \
  "$SB/pr/20260822-remoteapp-product-closure" \
  "$SB/Frontend/src/components/easynet" \
  "$SB/Frontend/src/lib/api" \
  "$SB/Frontend/src/store"

cp "$SCRIPT" "$SB/tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh"
cp "$HARNESS" "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh"
cp "$REPO_ROOT/tools/scripts/hub-api-readiness-preflight.sh" \
  "$SB/tools/scripts/hub-api-readiness-preflight.sh"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-permission-subject-e2e.sh" \
  "$SB/tools/scripts/host-remoteapp-permission-subject-e2e.sh"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-target-picker-freshness-e2e.sh" \
  "$SB/tools/scripts/host-remoteapp-target-picker-freshness-e2e.sh"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-decoded-frame-e2e.sh" \
  "$SB/tools/scripts/host-remoteapp-decoded-frame-e2e.sh"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-view-only-input-safety-e2e.sh" \
  "$SB/tools/scripts/host-remoteapp-view-only-input-safety-e2e.sh"
cp "$REPO_ROOT/plugins/remote-desktop/src/network.rs" \
  "$SB/plugins/remote-desktop/src/network.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/view_transport.rs" \
  "$SB/plugins/remote-desktop/src/view_transport.rs"
cp "$REPO_ROOT/docs/design/remoteapp-product-readiness-audit-2026-08-22.md" \
  "$SB/docs/design/remoteapp-product-readiness-audit-2026-08-22.md"
cp "$REPO_ROOT/pr/20260822-remoteapp-product-closure/02-evidence-audit.md" \
  "$SB/pr/20260822-remoteapp-product-closure/02-evidence-audit.md"
cp "$REPO_ROOT/../EasyNet/Frontend/src/components/easynet/DeviceMediaAccess.test.tsx" \
  "$SB/Frontend/src/components/easynet/DeviceMediaAccess.test.tsx"
cp "$REPO_ROOT/../EasyNet/Frontend/src/components/easynet/DeviceMediaAccess.tsx" \
  "$SB/Frontend/src/components/easynet/DeviceMediaAccess.tsx"
cp "$REPO_ROOT/../EasyNet/Frontend/src/lib/api/remote-desktop-protocol.ts" \
  "$SB/Frontend/src/lib/api/remote-desktop-protocol.ts"
cp "$REPO_ROOT/../EasyNet/Frontend/src/store/media-channel-store.ts" \
  "$SB/Frontend/src/store/media-channel-store.ts"
cp "$REPO_ROOT/../EasyNet/Frontend/src/store/media-channel-store.test.ts" \
  "$SB/Frontend/src/store/media-channel-store.test.ts"
chmod +x "$SB/tools/scripts/"*.sh

CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_ROOT="$SB" \
CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_FRONTEND_ROOT="$SB/Frontend" \
  bash "$SB/tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh" >/dev/null

cp "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh" \
  "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh.good"
perl -0pi -e 's/run_step hub-api-readiness-preflight run_hub_api_readiness_preflight/# hub api readiness preflight removed/g' \
  "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh"
if CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_ROOT="$SB" \
  CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_FRONTEND_ROOT="$SB/Frontend" \
  bash "$SB/tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh" >/dev/null 2>&1; then
  fail "checker accepted product-flow harness without Hub API readiness preflight step"
fi
mv "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh.good" \
  "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh"

cp "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh" \
  "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh.good"
perl -0pi -e 's/run_step hub-api-readiness-preflight run_hub_api_readiness_preflight\nrun_step product-runtime-readiness-preflight run_product_runtime_readiness_preflight/run_step product-runtime-readiness-preflight run_product_runtime_readiness_preflight\nrun_step hub-api-readiness-preflight run_hub_api_readiness_preflight/g' \
  "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh"
if CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_ROOT="$SB" \
  CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_FRONTEND_ROOT="$SB/Frontend" \
  bash "$SB/tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh" >/dev/null 2>&1; then
  fail "checker accepted product-flow harness that checks daemon before Hub API"
fi
mv "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh.good" \
  "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh"

cp "$SB/tools/scripts/hub-api-readiness-preflight.sh" \
  "$SB/tools/scripts/hub-api-readiness-preflight.sh.good"
perl -0pi -e 's#/api/v1/health#/debug-only-health#g' \
  "$SB/tools/scripts/hub-api-readiness-preflight.sh"
if CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_ROOT="$SB" \
  CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_FRONTEND_ROOT="$SB/Frontend" \
  bash "$SB/tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh" >/dev/null 2>&1; then
  fail "checker accepted Hub API preflight without canonical health endpoint"
fi
mv "$SB/tools/scripts/hub-api-readiness-preflight.sh.good" \
  "$SB/tools/scripts/hub-api-readiness-preflight.sh"

cp "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh" \
  "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh.good"
perl -0pi -e 's/run_step product-runtime-readiness-preflight run_product_runtime_readiness_preflight/# product runtime readiness preflight removed/g' \
  "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh"
if CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_ROOT="$SB" \
  CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_FRONTEND_ROOT="$SB/Frontend" \
  bash "$SB/tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh" >/dev/null 2>&1; then
  fail "checker accepted product-flow harness without product runtime readiness preflight step"
fi
mv "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh.good" \
  "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh"

cp "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh" \
  "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh.good"
perl -0pi -e 's/run_step product-runtime-readiness-preflight run_product_runtime_readiness_preflight\nrun_step frontend-typecheck run_frontend_tsc/run_step frontend-typecheck run_frontend_tsc\nrun_step product-runtime-readiness-preflight run_product_runtime_readiness_preflight/g' \
  "$SB/tools/scripts/frontend-remoteapp-product-flow-e2e.sh"
if CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_ROOT="$SB" \
  CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_FRONTEND_ROOT="$SB/Frontend" \
  bash "$SB/tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh" >/dev/null 2>&1; then
  fail "checker accepted product-flow harness that runs frontend before runtime readiness"
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

cp "$SB/Frontend/src/components/easynet/DeviceMediaAccess.test.tsx" \
  "$SB/Frontend/src/components/easynet/DeviceMediaAccess.test.tsx.good"
perl -0pi -e 's/target lost · target_not_found · refresh_targets/target lost/g' \
  "$SB/Frontend/src/components/easynet/DeviceMediaAccess.test.tsx"
if CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_ROOT="$SB" \
  CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_FRONTEND_ROOT="$SB/Frontend" \
  bash "$SB/tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh" >/dev/null 2>&1; then
  fail "checker accepted frontend coverage without target recovery action UI"
fi
mv "$SB/Frontend/src/components/easynet/DeviceMediaAccess.test.tsx.good" \
  "$SB/Frontend/src/components/easynet/DeviceMediaAccess.test.tsx"

cp "$SB/Frontend/src/components/easynet/DeviceMediaAccess.tsx" \
  "$SB/Frontend/src/components/easynet/DeviceMediaAccess.tsx.good"
perl -0pi -e 's/Refresh targets/Refresh inventory/g' \
  "$SB/Frontend/src/components/easynet/DeviceMediaAccess.tsx"
if CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_ROOT="$SB" \
  CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_FRONTEND_ROOT="$SB/Frontend" \
  bash "$SB/tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh" >/dev/null 2>&1; then
  fail "checker accepted frontend target recovery without Refresh targets CTA"
fi
mv "$SB/Frontend/src/components/easynet/DeviceMediaAccess.tsx.good" \
  "$SB/Frontend/src/components/easynet/DeviceMediaAccess.tsx"

cp "$SB/Frontend/src/components/easynet/DeviceMediaAccess.test.tsx" \
  "$SB/Frontend/src/components/easynet/DeviceMediaAccess.test.tsx.good"
perl -0pi -e 's#route host_only · no NAT/relay#route host_only#g' \
  "$SB/Frontend/src/components/easynet/DeviceMediaAccess.test.tsx"
if CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_ROOT="$SB" \
  CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_FRONTEND_ROOT="$SB/Frontend" \
  bash "$SB/tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh" >/dev/null 2>&1; then
  fail "checker accepted frontend coverage without host-only route visibility"
fi
mv "$SB/Frontend/src/components/easynet/DeviceMediaAccess.test.tsx.good" \
  "$SB/Frontend/src/components/easynet/DeviceMediaAccess.test.tsx"

cp "$SB/Frontend/src/components/easynet/DeviceMediaAccess.test.tsx" \
  "$SB/Frontend/src/components/easynet/DeviceMediaAccess.test.tsx.good"
perl -0pi -e 's/input scope display_global · pointer\+keyboard/input scope display_global/g' \
  "$SB/Frontend/src/components/easynet/DeviceMediaAccess.test.tsx"
if CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_ROOT="$SB" \
  CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_FRONTEND_ROOT="$SB/Frontend" \
  bash "$SB/tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh" >/dev/null 2>&1; then
  fail "checker accepted frontend coverage without input scope control visibility"
fi
mv "$SB/Frontend/src/components/easynet/DeviceMediaAccess.test.tsx.good" \
  "$SB/Frontend/src/components/easynet/DeviceMediaAccess.test.tsx"

cp "$SB/Frontend/src/components/easynet/DeviceMediaAccess.tsx" \
  "$SB/Frontend/src/components/easynet/DeviceMediaAccess.tsx.good"
perl -0pi -e 's/remoteDesktopInputScopeLabel\(session\)/undefined/g' \
  "$SB/Frontend/src/components/easynet/DeviceMediaAccess.tsx"
if CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_ROOT="$SB" \
  CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_FRONTEND_ROOT="$SB/Frontend" \
  bash "$SB/tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh" >/dev/null 2>&1; then
  fail "checker accepted frontend UI without input scope rendering"
fi
mv "$SB/Frontend/src/components/easynet/DeviceMediaAccess.tsx.good" \
  "$SB/Frontend/src/components/easynet/DeviceMediaAccess.tsx"

cp "$SB/Frontend/src/components/easynet/DeviceMediaAccess.test.tsx" \
  "$SB/Frontend/src/components/easynet/DeviceMediaAccess.test.tsx.good"
perl -0pi -e 's/media 18000kbps · 52\.5fps · drops 15 · backpressure 3/media 18000kbps/g' \
  "$SB/Frontend/src/components/easynet/DeviceMediaAccess.test.tsx"
if CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_ROOT="$SB" \
  CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_FRONTEND_ROOT="$SB/Frontend" \
  bash "$SB/tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh" >/dev/null 2>&1; then
  fail "checker accepted frontend coverage without media quality summary"
fi
mv "$SB/Frontend/src/components/easynet/DeviceMediaAccess.test.tsx.good" \
  "$SB/Frontend/src/components/easynet/DeviceMediaAccess.test.tsx"

cp "$SB/Frontend/src/components/easynet/DeviceMediaAccess.tsx" \
  "$SB/Frontend/src/components/easynet/DeviceMediaAccess.tsx.good"
perl -0pi -e 's/Retry session/Retry later/g' \
  "$SB/Frontend/src/components/easynet/DeviceMediaAccess.tsx"
if CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_ROOT="$SB" \
  CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_FRONTEND_ROOT="$SB/Frontend" \
  bash "$SB/tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh" >/dev/null 2>&1; then
  fail "checker accepted frontend recovery without Retry session CTA"
fi
mv "$SB/Frontend/src/components/easynet/DeviceMediaAccess.tsx.good" \
  "$SB/Frontend/src/components/easynet/DeviceMediaAccess.tsx"

cp "$SB/Frontend/src/store/media-channel-store.ts" \
  "$SB/Frontend/src/store/media-channel-store.ts.good"
perl -0pi -e 's/entry\.loading \|\| \(entry\.session && !remoteDesktopSessionTerminal\(entry\.session\)\)/entry.loading || entry.session/g' \
  "$SB/Frontend/src/store/media-channel-store.ts"
if CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_ROOT="$SB" \
  CHECK_REMOTEAPP_FRONTEND_PRODUCT_FLOW_FRONTEND_ROOT="$SB/Frontend" \
  bash "$SB/tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh" >/dev/null 2>&1; then
  fail "checker accepted store that blocks create after terminal session"
fi
mv "$SB/Frontend/src/store/media-channel-store.ts.good" \
  "$SB/Frontend/src/store/media-channel-store.ts"

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
