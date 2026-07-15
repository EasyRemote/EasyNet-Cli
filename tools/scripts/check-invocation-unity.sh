#!/usr/bin/env bash
# check-invocation-unity.sh
# ==========================
#
# CI gate for "Invocation = unique unit of execution".
# Documented under docs/design/invocation-unity-v1.md.
#
# The final project-structure-v1 layout routes live execution through
# daemon::invocation, daemon::boot::kernel, and Axon's LocalRuntime.
# The old daemon::kernel root must not return.
#
# Rule 1 (retired kernel root)
# --------------------------
# No final daemon code may import the old daemon::kernel namespace.
# The supported kernel home is daemon::boot::kernel.
#
# Rule 2 (execution cannot bypass daemon invocation)
# -------------------------------------------------
# schedule/runner.rs, loop_instance/runner.rs, permission/broker.rs
# must not call run_mission_inproc / Session::subscribe / dispatch
# directly. They construct/consume daemon invocation records and route
# through daemon::invocation / LocalRuntime adapters.
#
# Exit codes
#   0 — clean
#   1 — at least one violation

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

echo "== check-invocation-unity.sh =="
violations=0

# -- Rule 1: retired daemon kernel root must not return --------------
if [ -d "src/daemon" ]; then
    bad=$(grep -rnE 'crate::daemon::kernel\b' src/daemon \
        | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//' || true)
    if [ -n "$bad" ]; then
        echo "ERROR: retired daemon::kernel namespace referenced from active daemon code:"
        echo "$bad"
        echo "  Use daemon::boot::kernel or daemon::invocation modules instead."
        violations=$((violations + 1))
    fi
fi

# -- Rule 2: sub-services may not bypass daemon invocation ----------
#
#   * execution/schedule/    — must not call `run_mission_inproc`
#                              (the tick runner builds an
#                              Invocation and routes through daemon
#                              invocation dispatch).
#   * execution/loop_instance/ — must not call Session::subscribe
#                              or dispatch::send_to_agent directly.
#                              The loop controller emits one
#                              Invocation per body / verify step.
#   * execution/permission/  — broker may not be invoked from
#                              a legacy side-channel inside dispatch.rs.
#
# Whole-line `//` comments are excluded; subdir-scoped to keep
# the check from catching helper modules in other parts of the
# tree.
rule3_check() {
    local dir="$1"
    local pat="$2"
    [ -d "$dir" ] || return 0
    local bad
    bad=$(grep -rnE "$pat" "$dir" 2>/dev/null \
        | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//' || true)
    if [ -n "$bad" ]; then
        echo "ERROR: sub-service '$dir' bypasses daemon invocation dispatch:"
        echo "$bad"
        echo "  Build an Invocation and route through daemon::invocation;"
        echo "  do not reach for run_mission_inproc / Session::subscribe / send_to_agent."
        violations=$((violations + 1))
    fi
}
rule3_check "src/daemon/execution/schedule"      'run_mission_inproc'
rule3_check "src/daemon/execution/loop_instance" 'Session::subscribe|send_to_agent\(|run_mission_inproc'
rule3_check "src/daemon/execution/permission"    'run_mission_inproc'

if [ "$violations" -eq 0 ]; then
    echo "ok (no invocation-unity violations)"
    exit 0
fi
echo "FAILED: $violations violation(s)."
exit 1
