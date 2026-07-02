#!/usr/bin/env bash
# check-subservice-isolation.sh
# ==============================
#
# CI gate for the plan v10.2 Execution sub-service isolation rule.
# Documented under docs/design/daemon-layers-v1.md "Execution
# internal sub-service partition".
#
# Execution sub-services under `src/daemon/execution/<name>/` must
# not import each other directly. All cross-sub-service calls are
# routed through the Kernel (`src/runtime/kernel.rs`) so that a bug
# in one sub-service does not corrupt the others' state and so that
# the future isolation model (scheduler fairness, resource quotas)
# has a single chokepoint to instrument.
#
# Exit codes
#   0 — all sub-services are import-isolated from their siblings
#   1 — at least one sub-service reaches into another

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

echo "== check-subservice-isolation.sh =="

# Enumerate sub-services under src/daemon/execution.
if [ ! -d "src/daemon/execution" ]; then
    echo "ok (no execution/ tree; nothing to check)"
    exit 0
fi

subs=(session permission discuss schedule loop_instance)
violations=0

for self in "${subs[@]}"; do
    dir="src/daemon/execution/$self"
    [ -d "$dir" ] || continue
    for other in "${subs[@]}"; do
        [ "$self" = "$other" ] && continue
        # Match `crate::daemon::execution::<other>` or the super::
        # shortcut that bypasses the module path.
        pattern="crate::daemon::execution::${other}\b"
        offending=$(grep -rnE "$pattern" "$dir" || true)
        if [ -n "$offending" ]; then
            echo "ERROR: sub-service '$self' imports sibling '$other':"
            echo "$offending"
            echo "  All cross-sub-service calls go through the Kernel boundary."
            violations=$((violations + 1))
        fi
    done
done

if [ "$violations" -eq 0 ]; then
    echo "ok (no sub-service isolation violations)"
    exit 0
fi

echo "FAILED: $violations violation(s)."
exit 1
