#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

if [[ -e src/runtime/hosted_receipt.rs ]]; then
  fail "retired runtime::hosted_receipt compatibility shim still exists"
fi

if rg -n 'pub mod hosted_receipt|crate::runtime::hosted_receipt|runtime::hosted_receipt' src tests -g '*.rs'; then
  fail "hosted receipt callers must import axon_sdk::invocation::audit directly"
fi

if rg -n 'struct HostedAgentReceiptHeader|enum SigningModel|HostedReceiptError' src -g '*.rs'; then
  fail "CLI runtime must not redeclare Axon hosted receipt audit types"
fi

legacy_projection_roots=()
for root in src/daemon/execution/mission src/support src/ffi; do
  if [[ -d "$root" ]]; then
    legacy_projection_roots+=("$root")
  fi
done

if ((${#legacy_projection_roots[@]} > 0)) \
  && rg -n 'dispatch_receipt|receipt_header|HostedAgentReceiptHeader' "${legacy_projection_roots[@]}" -g '*.rs'; then
  fail "mission/support/FFI paths must not rebuild legacy hosted receipt headers"
fi

echo "check-hosted-receipt-axon-boundary: ok"
