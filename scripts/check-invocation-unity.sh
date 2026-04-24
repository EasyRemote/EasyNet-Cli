#!/usr/bin/env bash
# check-invocation-unity.sh
# ==========================
#
# CI gate for plan v10.3 C* "Invocation = unique unit of execution".
# Documented under docs/design/invocation-unity-v1.md.
#
# This script enforces two rules today; PR-INVOCATION-EXEC-UNITY
# strengthens it with three additional grep clauses once the
# schedule/loop/permission handlers have been collapsed onto
# Kernel::invoke.
#
# Rule 1 (syntactic unity)
# ------------------------
# IPC server / KernelApi trait / GatewayApi trait method signatures
# must not speak `args: serde_json::Value` / `args_json: ...` /
# `payload: serde_json::Value` tuples. They must speak domain types
# (Invocation, SessionId, PermissionRequest, ...) so the system-wide
# `invocation_id` stays the single key across layers.
#
# Rule 2 (unity entry point)
# --------------------------
# The ONLY place Kernel::invoke may be called from inside the
# runtime's execution layer is via the Kernel itself. Handlers
# (PR-ATTACH / PR-PERM / etc.) route execution entries through the
# Kernel, not by constructing a parallel dispatch.
#
# Rule 3 (reserved for PR-INVOCATION-EXEC-UNITY)
# ----------------------------------------------
# schedule/runner.rs, loop_instance/runner.rs, permission/broker.rs
# must not call run_mission_inproc / Session::subscribe / dispatch
# directly — they must construct Invocations and call
# Kernel::invoke. Left commented below until those runners land.
#
# Exit codes
#   0 — clean
#   1 — at least one violation

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "== check-invocation-unity.sh =="
violations=0

# ── Rule 1: forbid raw JSON fragments in syscall / gateway traits ─
# Only the trait definitions themselves are checked (files that
# declare the trait). Handler implementations may still operate
# on raw JSON internally because the `Invocation.args` field is
# typed `serde_json::Value` in v1.
for f in src/runtime/kernel_api.rs src/runtime/gateway_api.rs; do
    [ -f "$f" ] || continue
    bad=$(grep -nE 'args_json|payload: *(serde_json::)?Value|args: *(serde_json::)?Value' "$f" || true)
    if [ -n "$bad" ]; then
        echo "ERROR: trait definition at $f uses raw JSON payload fragments:"
        echo "$bad"
        echo "  Use Invocation or a typed domain object (see src/runtime/domain.rs)."
        violations=$((violations + 1))
    fi
done

# ── Rule 2: Kernel::invoke is the unity entry point ────────────────
# Detect the anti-pattern `Kernel::invoke(` in any sub-service other
# than the kernel itself. Sub-services must not "self-invoke" by
# reaching for Kernel; they return values to the Kernel that calls
# them. Flag if PR-INVOCATION-EXEC-UNITY leaks.
if [ -d "src/runtime/execution" ]; then
    # Look for actual call syntax `Kernel::invoke(` — excluding
    # backtick-wrapped prose in `// doc comments`. A simple rule
    # that works: reject the file when a non-comment line contains
    # `Kernel::invoke(`. `grep -v '^\s*//'` strips whole-line doc
    # comments; any call-site will fail that filter.
    bad=$(grep -rnE 'Kernel::invoke\(' src/runtime/execution \
        | grep -vE '^[^:]+:[0-9]+:\s*//' || true)
    if [ -n "$bad" ]; then
        echo "ERROR: Kernel::invoke is called from an Execution sub-service:"
        echo "$bad"
        echo "  The Kernel calls sub-services, not the other way around."
        violations=$((violations + 1))
    fi
fi

# ── Rule 3 (reserved for PR-INVOCATION-EXEC-UNITY) ────────────────
# TODO(PR-INVOCATION-EXEC-UNITY): once schedule/runner.rs, loop/
# runner.rs, permission/broker.rs land, add grep clauses that
# forbid:
#   - run_mission_inproc call-sites inside execution/schedule/
#   - direct Session::subscribe inside execution/loop_instance/
#   - broker side-channel inside dispatch.rs for execution/permission/
# The rules are disabled today because the files do not yet exist.

if [ "$violations" -eq 0 ]; then
    echo "ok (no invocation-unity violations)"
    exit 0
fi
echo "FAILED: $violations violation(s)."
exit 1
