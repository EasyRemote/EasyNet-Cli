#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_CORE_AGENT_MODULE_BOUNDARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
  printf 'check-core-agent-module-boundary: %s\n' "$1" >&2
  exit 1
}

CORE_MOD="src/core/mod.rs"
[[ -f "$CORE_MOD" ]] || fail "missing $CORE_MOD"

if rg -n 'pub use agent::(id as agent_id|spec as agent_spec)' "$CORE_MOD"; then
  fail "core must not keep pre-structure agent module compatibility aliases"
fi

if rg -n 'pre-structure Rust API paths' src --glob '!core/mod.rs'; then
  fail "production code must reference semantic agent owners under core::agent::{id,spec}"
fi

if rg -n 'crate::core::agent_(id|spec)|\bcore::agent_(id|spec)' src --glob '!core/mod.rs'; then
  fail "production callers must not use retired core agent module aliases"
fi

for token in \
  'pub mod agent;' \
  'pub mod domain;' \
  'pub mod identity;' \
  'pub mod ura;'; do
  if ! rg -n "$token" "$CORE_MOD" >/dev/null; then
    fail "core module missing canonical owner declaration: $token"
  fi
done

echo "check-core-agent-module-boundary: ok"
