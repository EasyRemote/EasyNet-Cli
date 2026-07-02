#!/usr/bin/env bash
# check-kernel-boundary.sh
# ==========================
#
# CI gate for the daemon-layers boundary documented under
# docs/design/daemon-layers-v1.md, src/daemon/control/mod.rs, and
# src/services/mod.rs.
#
# Five rules.
#
# Rules 1–2 cover services/* → runtime/*: the daemon's network-
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
#                            publish, execution, advertise,
#                            federation_client, abilities.
#
# (The former Rule 3 for `services/axon_bridge/` was retired on
# 2026-05-29 when that subtree moved to `runtime/axon_bridge/`. Its
# imports went almost entirely to `runtime/*` — keeping it under
# `services/` was a false hierarchy. Rule 6 below enforces the
# inverse direction for the moved tree.)
#
# Rule 3 (renumbered): Execution → GatewayApi only.
#
# Rules 4–5 cover the reverse direction (runtime/* → services/*).
# Most of `runtime/` is daemon-agnostic in-process plumbing; the
# only legitimate upward references are the few cells that hold
# cross-cutting state (trust anchor, hub-published ability store).
# Rule 4 rejects the retired `runtime/agents/` compatibility tree.
# Rule 5 bounds `runtime/axon_bridge/` with a tiny explicit allowlist
# so a new upward reference surfaces in code review.
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
#   * system              — build_registry() factory the convenience
#                           proxy constructor calls to materialise
#                           the local handler set
#   * local_runtime_invoker — peer of kernel_api: the LocalRuntime
#                           JSON adapter the proxy uses to translate
#                           Control-plane wire frames into Axon
#                           ability invocations. Lives one module
#                           down so the proxy's `crate::runtime::*`
#                           import surface stays small; the helpers
#                           themselves are syscall-boundary types
#                           (block_on_runtime, ability_frame_to_json,
#                           open_local_stream, …).
#   * hosted_receipt      — typed receipt-header shape carried in
#                           control-plane Result envelopes. Peer of
#                           invocation::Receipt; lives in its own
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
    control_allowed='kernel_api|invocation|invocation_target|domain|ability_dispatch|gateway_api|gateway|system|local_runtime_invoker|hosted_receipt|ability_names'
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
#   * ability, ability_descriptor, ability_names
#                         — descriptor versions, metadata keys,
#                           stable public ability names, canonical
#                           JSON bytes, and conformance metadata
#                           used to validate and project daemon
#                           Invocation requests.
#   * system_abilities    — terminal/file-transfer and governance
#                           handlers reached by the daemon
#                           Invocation server as in-process ability
#                           implementations.
#   * system_ability_catalog
#                         — descriptor/catalog snapshots used for
#                           session prelude advertisement and MCP
#                           profile projection.
#   * agent_ability_specs — hosted-agent ability specs advertised in
#                           prelude/publication flows.
#   * keyring             — sign/verify for cross-realm receipts
#                           and admission.
#   * publish             — federation.advertise_abilities backing
#                           store.
#   * local_invocation_identity
#                         — daemon-local caller identity used for
#                           self-authored forwarding, hosted-agent
#                           delegation, and local system invocations.
#   * failure_codes       — typed failure classifier used when
#                           projecting terminal daemon Invocation
#                           status into receipts.
#   * owner_projection    — read model for owner/ability projection,
#                           lease expiry, and advertised callable
#                           summaries used during session/presence
#                           updates.
#   * resources           — resource-ref helpers for filesystem
#                           transfer abilities; no handler policy
#                           is owned in the transport.
#   * join_connection_state
#                         — typed presence/join state snapshots
#                           recorded by session initiation.
#   * provisional_ura     — provisional identity helper for signed
#                           admission/bootstrap paths.
#   * federation_init     — daemon-side federation initialization
#                           decision surface referenced by boot.
#   * execution           — handle-level access to mcp_client / pty
#                           supervisors that handlers dispatch into.
#                           NOTE: this is the broadest exception
#                           and the one most likely to want
#                           narrowing in a follow-up PR.
#   * advertise           — federation.advertise_* helpers the boot
#                           sequence drives to register the daemon's
#                           Agents in the realm directory. Pure
#                           ability-call wrapper; no internal state.
#   * federation_client   — typed argument/response helpers for the
#                           four federation.* abilities the hub-
#                           profile Agent exposes. Wire-shape only.
#   * axon_bridge         — Axon-SDK glue (admission/dispatch
#                           shim, key resolver, runtime factory,
#                           hot agent registrar). The Invocation
#                           transport is the natural consumer: the gRPC server
#                           translates each wire frame into an
#                           Axon LocalRuntime invocation through
#                           the bridge.
#   * ability_wire        — daemon-facing wire profile registry for
#                           local bidi abilities. The transport
#                           reads codec metadata from this boundary;
#                           it does not inspect plugin packages or
#                           own execution policy.
#   * plugin_host         — daemon plugin runtime manager handle
#                           injected at boot so Invocation dispatch
#                           can execute already-loaded plugin-backed
#                           abilities. Package install/load policy
#                           remains in runtime/plugin_host.
#
# Forbidden by default: anything not on this list. Add with
# rationale here AND in docs/design/daemon-layers-v1.md.
if [ -d "src/daemon/invocation" ]; then
    serve_allowed='kernel_api|invocation|invocation_target|domain|ability_dispatch|gateway_api|gateway|system|local_runtime_invoker|hosted_receipt|ability|ability_descriptor|ability_names|system_abilities|system_ability_catalog|agent_ability_specs|keyring|publish|local_invocation_identity|failure_codes|owner_projection|resources|join_connection_state|provisional_ura|federation_init|execution|advertise|federation_client|axon_bridge|ability_wire|plugin_host'
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

# ── Rule 4 ────────────────────────────────────────────────────────
# runtime/agents/ was the old daemon-owned ability handler grouping.
# Handler implementations now live in `runtime/system_abilities`,
# descriptor/catalog projection lives in `runtime/system_ability_catalog`,
# and reusable execution engines live in `runtime/executors`.
if [ -d "src/runtime/agents" ]; then
    echo "ERROR: retired runtime agents directory exists."
    echo "  Use runtime/system_abilities, runtime/system_ability_catalog, or runtime/executors."
    violations=$((violations + 1))
fi

# ── Rule 5 ────────────────────────────────────────────────────────
# runtime/axon_bridge/ — the Axon-SDK glue. Lives in `runtime/`
# (formerly `services/`) precisely because its dependency
# direction is into runtime, not services. The only legitimate
# upward references are the trust-anchor cells the bridge reads
# at boot:
#
#   * trust_anchor_cell — `SharedTrustAnchor` typed cell the
#     `RealmTrustAnchorKeyResolver` snapshots on resolve.
#   * realm_trust_anchor — concrete `RealmTrustAnchor` /
#     `TrustedAgent` types used in unit tests; in production
#     the bridge depends only on the cell.
if [ -d "src/runtime/axon_bridge" ]; then
    bridge_up_allowed='trust_anchor_cell|realm_trust_anchor'
    bridge_up_files=$(find src/runtime/axon_bridge -name '*.rs' | sort)
    for f in $bridge_up_files; do
        awk '
            /^[[:space:]]*#\[cfg\(test\)\][[:space:]]*$/ { in_test = 1 }
            in_test { next }
            { print FILENAME ":" NR ":" $0 }
        ' "$f"
    done \
        | grep -E "crate::services::([a-zA-Z_][a-zA-Z0-9_]*)" \
        | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//' \
        | grep -vE "crate::services::(${bridge_up_allowed})\b" > /tmp/kb_bridge_up.$$ || true
    if [ -s /tmp/kb_bridge_up.$$ ]; then
        echo "ERROR: runtime/axon_bridge/ may not import these services modules:"
        cat /tmp/kb_bridge_up.$$
        echo "  Permitted: crate::services::{${bridge_up_allowed}}"
        violations=$((violations + 1))
    fi
    rm -f /tmp/kb_bridge_up.$$
fi

if [ "$violations" -eq 0 ]; then
    echo "ok (no kernel-boundary violations)"
    exit 0
fi

echo "FAILED: $violations rule(s) violated."
exit 1
