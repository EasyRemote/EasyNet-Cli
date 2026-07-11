#!/usr/bin/env bash
# check-ability-model-convergence.sh
# ===================================
#
# Guards the single daemon Ability model:
# - core stays free of executable/package manifest policy;
# - the descriptor package owns the only daemon CallMode enum;
# - manifest access scope and Page visibility remain distinct concepts;
# - plugins reuse the descriptor CallMode rather than declaring one.

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

if rg -n 'core::ability(::|_)' src -g '*.rs'; then
    echo "ERROR: callers still depend on the retired core ability-manifest namespace"
    violations=$((violations + 1))
fi

if [[ "$violations" -eq 0 ]]; then
    echo "ok (one daemon Ability model)"
    exit 0
fi

echo "FAILED: $violations convergence rule(s) violated."
exit 1
