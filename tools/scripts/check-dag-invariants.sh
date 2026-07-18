#!/usr/bin/env bash
# check-dag-invariants.sh
# ========================
#
# CI gate for plan v10.4 D1 — Invocation DAG structural invariants:
#
#   * D1 (acyclic):  no Invocation cites a future receipt as its
#                    causal_context. Wall-clock-time consistent.
#   * D2 (single callee sig per receipt):  each invocation_id has
#                    at most one Receipt; the Receipt has at most
#                    one callee_signature.
#
# Both are guaranteed by construction in v1: causal_context cites only
# prior receipts, daemon dispatch builds one Axon request/receipt
# lineage per Invocation, and v1 callee_signature is always None. This
# script is a guard rail: it runs the cargo unit-test suites that
# validate the invariants over synthesised invocation records and
# dispatch envelopes. A failure means a refactor broke the construction
# property.
#
# Why a wrapper script around cargo test
# --------------------------------------
# The other CI scripts in this directory are pure-grep checks, not
# Rust runs. Pinning the DAG check as a separate script keeps the
# CI pipeline visible: the file's existence (and its mention in
# docs/design/formal-model-v1.md §"Formal model tests") makes the
# invariant traceable to a runnable artifact.
#
# Exit codes
#   0 — invariants hold (cargo test passed)
#   1 — at least one invariant violated (cargo test failed)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

echo "== check-dag-invariants.sh =="
echo "Running daemon invocation DAG invariant test filters..."

run_filter() {
    local filter="$1"
    local tmp
    tmp="$(mktemp)"

    if ! cargo test --lib --quiet -- "$filter" >"$tmp" 2>&1; then
        echo "FAILED: cargo test filter failed: $filter"
        tail -40 "$tmp"
        rm -f "$tmp"
        exit 1
    fi

    if ! grep -Eq '^running [1-9][0-9]* tests' "$tmp"; then
        echo "FAILED: cargo test filter matched zero tests: $filter"
        cat "$tmp"
        rm -f "$tmp"
        exit 1
    fi

    echo "-- $filter"
    tail -20 "$tmp"
    rm -f "$tmp"
}

# Dispatch request tests exercise causal context projection on wire requests.
# Boot-kernel tests exercise the SDK request/finalized-receipt path. Axon
# dispatch-shim tests exercise receipt-bearing local runtime outcomes.
run_filter "daemon::invocation::dispatch::request"
run_filter "daemon::boot::kernel"
run_filter "daemon::axon_bridge::dispatch_shim"

echo "ok (D1 + D2 invariants hold for final daemon invocation construction)"
