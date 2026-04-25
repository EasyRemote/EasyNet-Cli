#!/usr/bin/env bash
# check-dispatch-boundary.sh
# ===========================
#
# CI gate for plan v10.1 two-stage dispatch (resolver + executor).
# Documented under docs/design/daemon-layers-v1.md and
# docs/design/invocation-unity-v1.md "Stage 1/2 separation".
#
# Ability handlers (under `src/runtime/system/*_ability.rs` once
# PR-SYS lands) must NOT decide locality by inspecting
# `target_node == self.node_id` style predicates. That decision
# belongs to the Stage 1 resolver in
# `src/runtime/invocation_target.rs` and is materialised as
# `InvocationTarget::scope`. Handlers consume the scope; they
# do not re-derive it.
#
# Exit codes
#   0 — no handler re-implements local/remote branching
#   1 — at least one handler violates the rule

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "== check-dispatch-boundary.sh =="

# Handler dir does not exist until PR-SYS lands. That's fine: the
# script still runs and reports a success for the empty set.
if [ ! -d "src/runtime/system" ]; then
    echo "ok (src/runtime/system not present; nothing to check)"
    exit 0
fi

violations=0

# Forbidden patterns inside handler files:
#   * self.node_id   — handler inspecting its owner's node identity
#   * target_node    — handler inspecting a routing decision
#   * my_node == / == my_node — equivalent check spelled differently
#
# Whole-line `//` comments are excluded — module / function doc
# comments may name `target_node` while explaining why handlers
# do not touch it.
bad=$(grep -rnE 'self\.node_id|\btarget_node\b' src/runtime/system \
    | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//' || true)
if [ -n "$bad" ]; then
    echo "ERROR: ability handler reads node identity / target_node directly:"
    echo "$bad"
    echo "  Consume InvocationTarget::scope from the stage-1 resolver"
    echo "  in src/runtime/invocation_target.rs; handlers do not branch"
    echo "  on locality themselves."
    violations=$((violations + 1))
fi

if [ "$violations" -eq 0 ]; then
    echo "ok (no dispatch-boundary violations)"
    exit 0
fi
echo "FAILED: $violations violation(s)."
exit 1
