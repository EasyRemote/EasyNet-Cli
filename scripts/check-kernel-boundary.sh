#!/usr/bin/env bash
# check-kernel-boundary.sh
# ==========================
#
# CI gate for the plan v10.5 R1 KernelApi + GatewayApi boundaries.
# Documented under docs/design/daemon-layers-v1.md.
#
# The Control layer (Rust files under `src/services/control/`) may
# only reach into the runtime through the syscall boundary — i.e.
# `crate::runtime::kernel_api`, `crate::runtime::invocation`,
# `crate::runtime::domain`. Anything else is forbidden: if the
# Control layer imports `crate::runtime::session`, a future refactor
# under `runtime::` would leak through Control and break the
# boundary documented at `daemon-layers-v1.md`.
#
# A second rule: the Execution layer (runtime::execution::*) must
# never import `crate::runtime::gateway` directly. It must go through
# the `GatewayApi` trait in `crate::runtime::gateway_api` so the
# concrete AxonGateway impl is swappable under tests and under
# future planners.
#
# Exit codes
#   0 — all rules satisfied
#   1 — at least one violation found (rule-specific message printed)
#
# Rule tuning
#   Adding a new permitted import requires updating the allowlist
#   arrays below and the corresponding rationale in
#   docs/design/daemon-layers-v1.md.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

violations=0

echo "== check-kernel-boundary.sh =="

# ── Rule 1 ────────────────────────────────────────────────────────
# Control layer may import only a narrow list of runtime submodules.
#
# The allowlist is deliberately short. If a new module belongs at the
# Control boundary, add it here + document the reasoning in the
# daemon-layers spec.
if [ -d "src/services/control" ]; then
    # Allowlist (final v1 set):
    #   * kernel_api          — syscall boundary trait
    #   * invocation          — Invocation/Receipt types
    #   * invocation_target   — stage-1 resolver shape
    #   * domain              — typed ids + handles
    #   * ability_dispatch    — stage-2 executor struct (interface
    #                           type the proxy consumes)
    #   * gateway_api         — Gateway trait (interface)
    #   * gateway             — NoopGateway used as the v1 default
    #                           when the proxy is constructed without
    #                           an injected Gateway
    #   * system              — `build_registry()` factory the
    #                           convenience proxy constructor calls
    #                           to materialise the local handler set
    #
    # Forbidden: execution::* sub-services, the concrete Kernel
    # struct, runtime::session/runtime::abilities (legacy paths
    # that pre-date the Kernel boundary).
    allowed='kernel_api|invocation|invocation_target|domain|ability_dispatch|gateway_api|gateway|system'
    # Find non-allowlisted `crate::runtime::<mod>` references in
    # non-test Rust code. Exclusions:
    #   * `^\s*//` — whole-line doc/code comments reference modules
    #     without importing them
    #   * files/blocks gated `#[cfg(test)]` — tests may use concrete
    #     types (e.g. NoopGateway, Kernel) to construct fixtures
    control_files=$(find src/services/control -name '*.rs' | sort)
    offending=""
    for f in $control_files; do
        # Strip lines inside `#[cfg(test)]` modules. We approximate
        # by dropping everything from a `#[cfg(test)]` line until
        # the end of file — for these skeletons the test module is
        # always the final one, which is the standard Rust layout.
        awk '
            /^[[:space:]]*#\[cfg\(test\)\][[:space:]]*$/ { in_test = 1 }
            in_test { next }
            { print FILENAME ":" NR ":" $0 }
        ' "$f"
    done \
        | grep -E "crate::runtime::([a-zA-Z_][a-zA-Z0-9_]*)" \
        | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//' \
        | grep -vE "crate::runtime::(${allowed})\b" > /tmp/kb_offending.$$ || true
    if [ -s /tmp/kb_offending.$$ ]; then
        echo "ERROR: Control layer is not allowed to import these runtime modules:"
        cat /tmp/kb_offending.$$
        echo "  Only crate::runtime::{${allowed}} is permitted."
        violations=$((violations + 1))
    fi
    rm -f /tmp/kb_offending.$$
fi

# ── Rule 2 ────────────────────────────────────────────────────────
# Execution layer must not reach into the concrete gateway impl.
# Execution → GatewayApi trait only.
if [ -d "src/runtime/execution" ]; then
    offending=$(grep -rnE "crate::runtime::gateway\b" src/runtime/execution \
        | grep -v "crate::runtime::gateway_api" || true)
    if [ -n "$offending" ]; then
        echo "ERROR: Execution layer must not import crate::runtime::gateway directly."
        echo "$offending"
        echo "  Use crate::runtime::gateway_api::GatewayApi trait instead."
        violations=$((violations + 1))
    fi
fi

if [ "$violations" -eq 0 ]; then
    echo "ok (no kernel-boundary violations)"
    exit 0
fi

echo "FAILED: $violations rule(s) violated."
exit 1
