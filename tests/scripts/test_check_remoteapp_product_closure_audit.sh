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
mkdir -p \
  "$SB/docs/design" \
  "$SB/pr/20260822-remoteapp-product-closure" \
  "$SB/tools/scripts" \
  "$SB/plugins/remote-desktop/src/handlers"
cp "$SCRIPT" "$SB/tools/scripts/check-remoteapp-product-closure-audit.sh"
cp "$REPO_ROOT/tools/scripts/remoteapp-cross-device-product-smoke.sh" "$SB/tools/scripts/remoteapp-cross-device-product-smoke.sh"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-session-timeout-e2e.sh" "$SB/tools/scripts/host-remoteapp-session-timeout-e2e.sh"
cp "$REPO_ROOT/docs/design/remoteapp-targeted-session-spec.md" "$SB/docs/design/remoteapp-targeted-session-spec.md"
cp "$REPO_ROOT/docs/design/remoteapp-product-readiness-audit-2026-08-22.md" "$SB/docs/design/remoteapp-product-readiness-audit-2026-08-22.md"
cp "$REPO_ROOT/docs/design/remoteapp-product-readiness-matrix.json" "$SB/docs/design/remoteapp-product-readiness-matrix.json"
cp "$REPO_ROOT/pr/20260822-remoteapp-product-closure/02-evidence-audit.md" "$SB/pr/20260822-remoteapp-product-closure/02-evidence-audit.md"
cp "$REPO_ROOT/plugins/remote-desktop/src/session.rs" "$SB/plugins/remote-desktop/src/session.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/view.rs" "$SB/plugins/remote-desktop/src/view.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/event_log.rs" "$SB/plugins/remote-desktop/src/event_log.rs"
cp "$REPO_ROOT/plugins/remote-desktop/src/handlers/mod.rs" "$SB/plugins/remote-desktop/src/handlers/mod.rs"

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

cp "$REPO_ROOT/docs/design/remoteapp-product-readiness-audit-2026-08-22.md" "$SB/docs/design/remoteapp-product-readiness-audit-2026-08-22.md"
perl -0pi -e 's#does not prove real OS window/application capture#proves real OS window/application capture#g' \
  "$SB/tools/scripts/remoteapp-cross-device-product-smoke.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-cross-device-gate.out 2>&1; then
  fail "checker accepted cross-device smoke without product non-claims"
fi
grep -q "cross-device smoke must preserve product non-claims" /tmp/check-remoteapp-product-closure-cross-device-gate.out || \
  fail "expected cross-device smoke non-claim failure"
cp "$REPO_ROOT/tools/scripts/remoteapp-cross-device-product-smoke.sh" "$SB/tools/scripts/remoteapp-cross-device-product-smoke.sh"

perl -0pi -e 's#remote_desktop\.show_session#remote_desktop.status#g' \
  "$SB/tools/scripts/host-remoteapp-session-timeout-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-timeout-show.out 2>&1; then
  fail "checker accepted session timeout E2E without public show_session observation"
fi
grep -q "session timeout E2E must observe timeout through public show_session" /tmp/check-remoteapp-product-closure-timeout-show.out || \
  fail "expected timeout show_session failure"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-session-timeout-e2e.sh" "$SB/tools/scripts/host-remoteapp-session-timeout-e2e.sh"

perl -0pi -e 's#terminal_receipt\.reason_code#terminal_receipt.reason#g' \
  "$SB/tools/scripts/host-remoteapp-session-timeout-e2e.sh"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-timeout-receipt.out 2>&1; then
  fail "checker accepted session timeout E2E without terminal_receipt.reason_code validation"
fi
grep -q "session timeout E2E must inspect timeout terminal_receipt.reason_code" /tmp/check-remoteapp-product-closure-timeout-receipt.out || \
  fail "expected timeout receipt reason_code failure"
cp "$REPO_ROOT/tools/scripts/host-remoteapp-session-timeout-e2e.sh" "$SB/tools/scripts/host-remoteapp-session-timeout-e2e.sh"

python3 - "$SB/docs/design/remoteapp-product-readiness-matrix.json" <<'PY'
import json
import sys

path = sys.argv[1]
matrix = json.load(open(path, encoding="utf-8"))
matrix["product_complete"] = True
matrix["status"] = "complete"
json.dump(matrix, open(path, "w", encoding="utf-8"), indent=2)
PY
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-matrix-complete.out 2>&1; then
  fail "checker accepted matrix that claims product completion"
fi
grep -q "product_complete must be false" /tmp/check-remoteapp-product-closure-matrix-complete.out || \
  fail "expected product_complete matrix failure"

cp "$REPO_ROOT/docs/design/remoteapp-product-readiness-matrix.json" "$SB/docs/design/remoteapp-product-readiness-matrix.json"
python3 - "$SB/docs/design/remoteapp-product-readiness-matrix.json" <<'PY'
import json
import sys

path = sys.argv[1]
matrix = json.load(open(path, encoding="utf-8"))
matrix["requirements"] = [
    row for row in matrix["requirements"] if row["id"] != "frontend_lifecycle"
]
json.dump(matrix, open(path, "w", encoding="utf-8"), indent=2)
PY
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-matrix-missing.out 2>&1; then
  fail "checker accepted matrix without frontend lifecycle row"
fi
grep -q "missing requirement ids: frontend_lifecycle" /tmp/check-remoteapp-product-closure-matrix-missing.out || \
  fail "expected missing frontend lifecycle matrix failure"

cp "$REPO_ROOT/docs/design/remoteapp-product-readiness-matrix.json" "$SB/docs/design/remoteapp-product-readiness-matrix.json"
perl -0pi -e 's/"terminal_receipt": session\.terminal_receipt\(\),//' \
  "$SB/plugins/remote-desktop/src/view.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-terminal-view.out 2>&1; then
  fail "checker accepted session view without terminal_receipt projection"
fi
grep -q "session view must expose terminal_receipt" /tmp/check-remoteapp-product-closure-terminal-view.out || \
  fail "expected missing terminal_receipt view failure"

cp "$REPO_ROOT/plugins/remote-desktop/src/view.rs" "$SB/plugins/remote-desktop/src/view.rs"
perl -0pi -e 's/idempotent end_session must return the original terminal receipt/idempotent end_session does not check terminal receipt/' \
  "$SB/plugins/remote-desktop/src/handlers/mod.rs"
if (cd "$SB" && CHECK_REMOTEAPP_PRODUCT_CLOSURE_ROOT="$SB" bash tools/scripts/check-remoteapp-product-closure-audit.sh) >/tmp/check-remoteapp-product-closure-terminal-idempotent.out 2>&1; then
  fail "checker accepted end_session without terminal receipt idempotency proof"
fi
grep -q "end_session tests must prove idempotent close returns the original terminal receipt" /tmp/check-remoteapp-product-closure-terminal-idempotent.out || \
  fail "expected missing terminal receipt idempotency failure"

echo "test_check_remoteapp_product_closure_audit: ok"
