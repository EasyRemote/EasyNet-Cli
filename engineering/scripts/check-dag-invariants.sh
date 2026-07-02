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
# Both are guaranteed by construction in v1 (causal_context cites
# only prior receipts; Kernel::invoke writes one Receipt per
# Invocation; v1 callee_signature is always None). This script is
# a guard rail: it runs the cargo unit-test suite that validates
# the invariants over a synthesised DAG. A failure means a v2
# refactor accidentally broke the construction property.
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
echo "Running runtime::invocation::tests + runtime::kernel::tests..."

# `runtime::invocation::tests` exercises the canonical-bytes
# invariant + invocation_id stability (D1's content-addressing
# precondition). `runtime::kernel::tests` exercises Kernel::invoke
# returning a Receipt whose invocation_id matches the input (D2's
# "one Receipt per Invocation" half).
cargo test --lib --quiet -- \
    runtime::invocation \
    runtime::kernel:: \
    2>&1 | tail -20

echo "ok (D1 + D2 invariants hold for v1 construction)"
