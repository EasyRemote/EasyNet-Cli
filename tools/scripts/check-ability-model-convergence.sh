#!/usr/bin/env bash
# check-ability-model-convergence.sh
# ===================================
#
# Guards the single daemon Ability model:
# - core stays free of executable/package manifest policy;
# - the descriptor package owns the only daemon CallMode enum;
# - manifest access scope and Page visibility remain distinct concepts;
# - plugins reuse the descriptor CallMode rather than declaring one.
# - authority and registration APIs cannot re-introduce implicit-owner or
#   parallel-record compatibility models.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

echo "== check-ability-model-convergence.sh =="
violations=0

if rg -n 'AbilityManifest|AbilityExec|BootSpec|HealthSpec|ManifestAccessScope' src/core -g '*.rs'; then
    echo "ERROR: core contains daemon executable-manifest semantics"
    violations=$((violations + 1))
fi

call_mode_defs="$(rg -l '^pub enum CallMode \{$' src -g '*.rs' || true)"
if [[ "$call_mode_defs" != "src/daemon/ability/descriptors/mod.rs" ]]; then
    echo "ERROR: daemon transport mode must be defined only by ability descriptors:"
    echo "$call_mode_defs"
    violations=$((violations + 1))
fi

if rg -n '\bPluginCallMode\b|^pub enum Visibility' src/daemon/plugins src/daemon/ability/builtins/resources/pages -g '*.rs'; then
    echo "ERROR: plugin or pages code declares an overloaded Ability visibility/mode type"
    violations=$((violations + 1))
fi

if rg -n 'pub use crate::daemon::ability::CallMode' src/daemon/plugins/manifest.rs; then
    echo "ERROR: plugin manifest must consume descriptor CallMode, not re-export it"
    violations=$((violations + 1))
fi

if rg -n 'pub use manifest::\{[^}]*\bCallMode\b' src/daemon/plugins/mod.rs; then
    echo "ERROR: plugin module must not re-export descriptor CallMode through plugins::*"
    violations=$((violations + 1))
fi

if rg -n 'use crate::daemon::plugins::manifest::(\{[^}]*\bCallMode\b|CallMode\b)' src/daemon/plugins -g '*.rs'; then
    echo "ERROR: plugin code must import CallMode from daemon::ability, not manifest"
    violations=$((violations + 1))
fi

if rg -n 'use crate::daemon::plugins::(\{[^}]*\bCallMode\b|CallMode\b)' src plugins -g '*.rs'; then
    echo "ERROR: plugin consumers must import CallMode from daemon::ability, not plugins"
    violations=$((violations + 1))
fi

if rg -n 'core::ability(::|_)' src -g '*.rs'; then
    echo "ERROR: callers still depend on the retired core ability-manifest namespace"
    violations=$((violations + 1))
fi

if rg -n '\bAbilityDescriptorRecord\b|\bAuthorityBindingRecord\b' src -g '*.rs'; then
    echo "ERROR: retired parallel Ability/authority record model returned"
    violations=$((violations + 1))
fi

if rg -n 'pub const NODE_(LIST|DESCRIBE|REMOVE):' src/daemon/ability/names/federation.rs; then
    echo "ERROR: device lifecycle ability names must be owned by names::device_control, not names::federation"
    violations=$((violations + 1))
fi

if rg -n 'pub const NODE_LIST|local_rpc!\("node\.list"|register_rpc_with_spec\([^)]*NODE_LIST|ABILITY_LIST_NODES' \
    src/daemon -g '*.rs'; then
    echo "ERROR: retired node.list fleet route must not re-enter the daemon ability model"
    violations=$((violations + 1))
fi

if rg -n 'names::federation::NODE_(LIST|DESCRIBE|REMOVE)|federation_names::NODE_(LIST|DESCRIBE|REMOVE)|federation::NODE_(LIST|DESCRIBE|REMOVE)' src -g '*.rs'; then
    echo "ERROR: device lifecycle callers must import node.* constants from names::device_control"
    violations=$((violations + 1))
fi

if rg -n '^\s*pub fn (register_rpc|register_stream|register_bidi|register_rpc_with_envelope|register_stream_with_envelope|register_bidi_with_envelope)\(' \
    src/daemon/ability/dispatch.rs; then
    echo "ERROR: catalog registration must require an explicit owner"
    violations=$((violations + 1))
fi

if rg -n '\bbuild_registry_with_services\(' src tests -g '*.rs'; then
    echo "ERROR: retired infallible catalog assembly entry point returned"
    violations=$((violations + 1))
fi

if rg -n -U 'load_agents\(\)[[:space:]]*\.unwrap_or' \
    src/daemon/ability/catalog/build.rs; then
    echo "ERROR: catalog assembly must propagate durable Agent registry failures"
    violations=$((violations + 1))
fi

if rg -n 'fallback[[:space:]]*=[[:space:]]*"empty_service"' \
    src/daemon/ability/catalog/build.rs; then
    echo "ERROR: invalid daemon service configuration must not become an empty provider"
    violations=$((violations + 1))
fi

if [[ "$violations" -eq 0 ]]; then
    echo "ok (one daemon Ability model)"
    exit 0
fi

echo "FAILED: $violations convergence rule(s) violated."
exit 1
