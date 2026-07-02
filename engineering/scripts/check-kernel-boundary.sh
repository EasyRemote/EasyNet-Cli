#!/usr/bin/env bash
# check-kernel-boundary.sh
# ==========================
#
# CI gate for the daemon-layers boundary documented under
# docs/design/daemon-layers-v1.md and src/daemon/control/mod.rs.
#
# Five rules.
#
# Rules 1–2 cover daemon/* → runtime/*: the daemon's network-
# facing surfaces (control plane, gRPC Invocation server) are
# allowed to reach into a bounded set of runtime modules because
# every one of those concerns surfaces on a wire RPC.
#
#   * daemon/control/    — wire adapter for the local Control
#                            plane. Speaks to LocalRuntime through
#                            one resolver + the syscall boundary
#                            types. The narrowest allowlist.
#   * daemon/invocation/ — the daemon's gRPC Invocation server.
#                            Wider allowlist; covers ability
#                            dispatch, agents catalog, keyring,
#                            publish, execution, abilities.
#
# (The former Rule 3 for `services/axon_bridge/` was retired on
# 2026-05-29 when that subtree moved out of `services/`. It now lives
# in `daemon/axon_bridge/`, which is the Clean Final daemon-owned Axon
# SDK adapter home.)
#
# Rule 3 (renumbered): Execution → GatewayApi only.
#
# Rules 4–5 reject retired runtime compatibility trees.
# Rule 4 rejects the retired `runtime/agents/` compatibility tree.
# Rule 5 rejects the retired `runtime/axon_bridge/` adapter tree.
#
# Exit codes
#   0 — all rules satisfied
#   1 — at least one violation found
#
# Rule tuning
#   Adding a new permitted import requires updating both the allowlist
#   array below AND the corresponding rationale in
#   docs/design/daemon-layers-v1.md. The CI grep for the rationale
#   exists so we don't drift the spec.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

violations=0

echo "== check-kernel-boundary.sh =="

# ── Rule 1 ────────────────────────────────────────────────────────
# daemon/control/ — narrow syscall surface.
#
# Allowlist (final v1 set):
#   * domain              — typed ids + handles
#   * system              — build_registry() factory the convenience
#                           proxy constructor calls to materialise
#                           the local handler set
#   * hosted_receipt      — typed receipt-header shape carried in
#                           control-plane Result envelopes. Peer of
#                           daemon::invocation::runtime_record::Receipt; lives in its own
#                           module because the §A12 hosted-vs-self
#                           distinction is independent of the
#                           Invocation lifecycle types.
#   * ability_names       — stable ability-name constants used by
#                           control discovery/watch bootstrap
#                           responses. Constants only; no handler
#                           ownership crosses the boundary.
#
# Forbidden: execution::* sub-services, the concrete Kernel
# struct, and runtime::session (a legacy path that pre-dates the
# Kernel boundary).
if [ -d "src/daemon/control" ]; then
    control_allowed='domain|system|hosted_receipt|ability_names'
    control_files=$(find src/daemon/control -name '*.rs' | sort)
    for f in $control_files; do
        awk '
            /^[[:space:]]*#\[cfg\(test\)\][[:space:]]*$/ { in_test = 1 }
            in_test { next }
            { print FILENAME ":" NR ":" $0 }
        ' "$f"
    done \
        | grep -E "crate::runtime::([a-zA-Z_][a-zA-Z0-9_]*)" \
        | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//' \
        | grep -vE "crate::runtime::(${control_allowed})\b" > /tmp/kb_control.$$ || true
    if [ -s /tmp/kb_control.$$ ]; then
        echo "ERROR: daemon/control/ may not import these runtime modules:"
        cat /tmp/kb_control.$$
        echo "  Permitted: crate::runtime::{${control_allowed}}"
        violations=$((violations + 1))
    fi
    rm -f /tmp/kb_control.$$
fi

# ── Rule 2 ────────────────────────────────────────────────────────
# daemon/invocation/ — daemon's gRPC Invocation server.
#
# Wider permitted set, listed explicitly so a new import surfaces
# in code review rather than slipping into an unchecked tree.
#
# Allowlist:
#   * The full control set (rule 1) — every syscall type is also
#     legitimately used here.
#   * agent_ability_specs — hosted-agent ability specs advertised in
#                           prelude/publication flows.
#   * keyring             — sign/verify for cross-realm receipts
#                           and admission.
#   * failure_codes       — typed failure classifier used when
#                           projecting terminal daemon Invocation
#                           status into receipts.
#   * join_connection_state
#                         — typed presence/join state snapshots
#                           recorded by session initiation.
#   * provisional_ura     — provisional identity helper for signed
#                           admission/bootstrap paths.
# Forbidden by default: anything not on this list. Add with
# rationale here AND in docs/design/daemon-layers-v1.md.
if [ -d "src/daemon/invocation" ]; then
    serve_allowed='domain|system|hosted_receipt|agent_ability_specs|keyring|failure_codes|join_connection_state|provisional_ura'
    serve_files=$(find src/daemon/invocation -name '*.rs' | sort)
    for f in $serve_files; do
        awk '
            /^[[:space:]]*#\[cfg\(test\)\][[:space:]]*$/ { in_test = 1 }
            in_test { next }
            { print FILENAME ":" NR ":" $0 }
        ' "$f"
    done \
        | grep -E "crate::runtime::([a-zA-Z_][a-zA-Z0-9_]*)" \
        | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//' \
        | grep -vE "crate::runtime::(${serve_allowed})\b" > /tmp/kb_serve.$$ || true
    if [ -s /tmp/kb_serve.$$ ]; then
        echo "ERROR: daemon/invocation/ may not import these runtime modules:"
        cat /tmp/kb_serve.$$
        echo "  Permitted: crate::runtime::{${serve_allowed}}"
        violations=$((violations + 1))
    fi
    rm -f /tmp/kb_serve.$$
fi

# ── Rule 3 ────────────────────────────────────────────────────────
# Execution layer must not reach into the concrete gateway impl.
# Execution → GatewayApi trait only.
if [ -d "src/daemon/execution" ]; then
    offending=$(grep -rnE "crate::daemon::federation::gateway\b" src/daemon/execution \
        | grep -v "crate::daemon::federation::gateway_api" || true)
    if [ -n "$offending" ]; then
        echo "ERROR: Execution layer must not import crate::daemon::federation::gateway directly."
        echo "$offending"
        echo "  Use crate::daemon::federation::gateway_api::GatewayApi trait instead."
        violations=$((violations + 1))
    fi
fi

# ── Rule 4 ────────────────────────────────────────────────────────
# runtime/agents/ was the old daemon-owned ability handler grouping.
# Handler implementations now live in `daemon/ability/builtins`,
# descriptor/catalog projection lives in `daemon/ability/catalog`,
# and reusable execution engines live in `runtime/executors`.
if [ -d "src/runtime/agents" ]; then
    echo "ERROR: retired runtime agents directory exists."
    echo "  Use daemon/ability/builtins, daemon/ability/catalog, or runtime/executors."
    violations=$((violations + 1))
fi

# ── Rule 5 ────────────────────────────────────────────────────────
# runtime/axon_bridge/ is retired. Axon SDK adapters are daemon-owned
# because they wire daemon trust, invocation, and built-in ability
# registration into Axon LocalRuntime.
if [ -d "src/runtime/axon_bridge" ]; then
    echo "ERROR: retired runtime axon_bridge directory exists."
    echo "  Use daemon/axon_bridge for Axon SDK adapters."
    violations=$((violations + 1))
fi

if [ "$violations" -eq 0 ]; then
    echo "ok (no kernel-boundary violations)"
    exit 0
fi

echo "FAILED: $violations rule(s) violated."
exit 1
