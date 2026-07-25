#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

WIRE="src/daemon/invocation/dispatch/invocation_wire.rs"
[[ -f "$WIRE" ]] || fail "missing $WIRE"

if ! rg -n 'enum EntityRefKindResolution' "$WIRE" >/dev/null; then
  fail "invocation wire must resolve EntityRefKind through EntityRefKindResolution"
fi

for method in \
  'fn from_ura\(ura: &str\) -> anyhow::Result<Self>' \
  'fn protobuf_kind\(self\) -> EntityRefKind'
do
  if ! rg -n "$method" "$WIRE" >/dev/null; then
    fail "EntityRefKindResolution is missing required method pattern: $method"
  fi
done

for state in 'Agent' 'Ability' 'Device' 'Resource' 'Session' 'Continuation' 'StateObject'; do
  if ! rg -n "^[[:space:]]*$state," "$WIRE" >/dev/null; then
    fail "EntityRefKindResolution is missing state: $state"
  fi
done

if ! rg -n 'EntityRefKindResolution::from_ura\(&ura\)\?\.protobuf_kind\(\)' "$WIRE" >/dev/null; then
  fail "try_entity_ref must project protobuf kind through EntityRefKindResolution"
fi

if ! rg -n 'fn top_level_subject_resolution\(ura: &str\) -> Option<EntityRefKindResolution>' "$WIRE" >/dev/null; then
  fail "top-level subject form handling must return EntityRefKindResolution"
fi

if rg -n 'infer_entity_ref_kind|top_level_subject_entity_kind' "$WIRE"; then
  fail "invocation wire still has obsolete entity-ref inference helper(s)"
fi

if rg -n 'infer entity ref|inferred entity ref|subject kind fallback|kind fallback' "$WIRE"; then
  fail "invocation wire still uses fallback/inference vocabulary for EntityRef kind resolution"
fi
