#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

if [[ -e src/runtime/hosted_receipt.rs ]]; then
  fail "retired runtime::hosted_receipt compatibility shim still exists"
fi

if rg -n 'pub mod hosted_receipt|crate::runtime::hosted_receipt|runtime::hosted_receipt' src tests -g '*.rs'; then
  fail "hosted receipt callers must import easynet_axon::invocation::audit directly"
fi

if rg -n 'struct HostedAgentReceiptHeader|enum SigningModel|HostedReceiptError' src/runtime -g '*.rs'; then
  fail "CLI runtime must not redeclare Axon hosted receipt audit types"
fi

echo "check-hosted-receipt-axon-boundary: ok"
