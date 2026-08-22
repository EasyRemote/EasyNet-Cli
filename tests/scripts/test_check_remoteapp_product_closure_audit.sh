#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-remoteapp-product-closure-audit.sh"

fail() {
  printf 'test_check_remoteapp_product_closure_audit: %s\n' "$1" >&2
  exit 1
}

"$SCRIPT"

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT
mkdir -p "$SB/docs/design" "$SB/pr/20260822-remoteapp-product-closure" "$SB/tools/scripts"
cp "$SCRIPT" "$SB/tools/scripts/check-remoteapp-product-closure-audit.sh"
cp "$REPO_ROOT/docs/design/remoteapp-targeted-session-spec.md" "$SB/docs/design/remoteapp-targeted-session-spec.md"
cp "$REPO_ROOT/docs/design/remoteapp-product-readiness-audit-2026-08-22.md" "$SB/docs/design/remoteapp-product-readiness-audit-2026-08-22.md"
cp "$REPO_ROOT/pr/20260822-remoteapp-product-closure/02-evidence-audit.md" "$SB/pr/20260822-remoteapp-product-closure/02-evidence-audit.md"

perl -0pi -e 's/full RemoteApp product closure incomplete as of 2026-08-22/implemented; full acceptance verified 2026-08-16/' \
  "$SB/docs/design/remoteapp-targeted-session-spec.md"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-status.out 2>&1; then
  fail "checker accepted targeted-session SPEC that claims full product acceptance"
fi
grep -q "must not claim full product acceptance" /tmp/check-remoteapp-product-closure-status.out || \
  fail "expected status misclaim failure"

cp "$REPO_ROOT/docs/design/remoteapp-targeted-session-spec.md" "$SB/docs/design/remoteapp-targeted-session-spec.md"
perl -0pi -e 's#Cross-device E2E smoke/regression exists beyond local provider boundary#Cross-device local-only smoke#' \
  "$SB/docs/design/remoteapp-product-readiness-audit-2026-08-22.md"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-cross-device.out 2>&1; then
  fail "checker accepted audit without cross-device product proof row"
fi
grep -q "cross-device proof" /tmp/check-remoteapp-product-closure-cross-device.out || \
  fail "expected cross-device audit failure"

echo "test_check_remoteapp_product_closure_audit: ok"
