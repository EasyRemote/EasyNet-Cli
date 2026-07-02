#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SCRIPT="$REPO_ROOT/engineering/scripts/check-hosted-receipt-axon-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p "$SB/engineering/scripts" "$SB/src/runtime" "$SB/tests"
cp "$SCRIPT" "$SB/engineering/scripts/check-hosted-receipt-axon-boundary.sh"
printf 'pub mod dispatch_receipt;\n' > "$SB/src/runtime/mod.rs"
printf 'use easynet_axon::invocation::audit::HostedAgentReceiptHeader;\n' \
  > "$SB/src/runtime/dispatch_receipt.rs"

(
  cd "$SB"
  bash engineering/scripts/check-hosted-receipt-axon-boundary.sh
) >/dev/null || fail "happy path should pass"

printf 'pub use easynet_axon::invocation::audit::HostedAgentReceiptHeader;\n' \
  > "$SB/src/runtime/hosted_receipt.rs"
set +e
(
  cd "$SB"
  bash engineering/scripts/check-hosted-receipt-axon-boundary.sh
) >/tmp/check-hosted-receipt-boundary.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "retired shim file should exit 1 (got $rc)"
rm "$SB/src/runtime/hosted_receipt.rs"

printf 'pub mod hosted_receipt;\n' >> "$SB/src/runtime/mod.rs"
set +e
(
  cd "$SB"
  bash engineering/scripts/check-hosted-receipt-axon-boundary.sh
) >/tmp/check-hosted-receipt-boundary.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "retired module export should exit 1 (got $rc)"
perl -0pi -e 's/pub mod hosted_receipt;\n//' "$SB/src/runtime/mod.rs"

printf 'use crate::runtime::hosted_receipt::SigningModel;\n' \
  >> "$SB/src/runtime/dispatch_receipt.rs"
set +e
(
  cd "$SB"
  bash engineering/scripts/check-hosted-receipt-axon-boundary.sh
) >/tmp/check-hosted-receipt-boundary.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "retired module import should exit 1 (got $rc)"

echo "test_check_hosted_receipt_axon_boundary.sh: all cases passed"
