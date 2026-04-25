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

# Forbidden patterns inside handler files (locality-derivation):
#   * self.node_id        — handler inspecting its owner's node identity
#   * target_node ==      — handler comparing routing decision
#   * == target_node      — same, spelled the other way around
#   * my_node ==          — equivalent check via a renamed local
#
# `target_node` is permitted as a domain field name (e.g. inside a
# `ScheduleEntry { target_node: NodeId::new(arg), ... }` constructor
# or as a JSON arg key) — that records "where the scheduled fire
# should land", which is a domain attribute, not a dispatch decision.
# The dispatch decision is what we ban: any comparison that branches
# on locality.
#
# Whole-line `//` comments are excluded — module / function doc
# comments may name `target_node` while explaining why handlers
# do not touch it.
bad=$(grep -rnE 'self\.node_id|\btarget_node[[:space:]]*==|==[[:space:]]*\btarget_node|\bmy_node[[:space:]]*==' src/runtime/system \
    | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//' || true)
if [ -n "$bad" ]; then
    echo "ERROR: ability handler branches on locality directly:"
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
