# Execution handler projection convergence

## Goal

Remove the implicit ability-name handler merge inside `ExecutionIndex`. Handler
lookup may remain source-compatible at the public API boundary, but the catalog
must not synthesize a runnable `RuntimeHandlerSet` by filling missing slots from
multiple authority-scoped rows.

## Root abstraction problem

`ExecutionIndex::handlers_for_ability` merges static rows first and then dynamic
rows by bare ability name. That is a compatibility-shaped projection: it turns
separate authority-scoped execution rows into one synthetic handler set.
Runtime binding and descriptor proof ownership are authority/mode scoped, so a
synthetic cross-row handler set can hide a missing or ambiguous authority path.

## Invariants

1. `ExecutionIndex` remains keyed by `ControlPlaneAbilityKey`.
2. Runtime sync reads handlers by exact execution key, not by ability-name merge.
3. Ability-name public lookup returns a handler only when the requested handler
   slot is unique.
4. Ambiguous same-name handler slots fail closed as `None` / `false` at the
   source-compatible API boundary.
5. Dynamic/static registration lifecycle and control-plane transaction behavior
   remain unchanged.

## Implementation order

1. Add slot-specific unique projection helpers to `ExecutionIndex`.
2. Migrate routeability and `resolve_*` to slot-specific projections.
3. Migrate runtime sync to `handlers_for_key`.
4. Remove `handlers_for_ability` and `RuntimeHandlerSet::fill_missing_from`.
5. Add tests and convergence gates that reject ability-name handler-set merge.
6. Verify targeted tests, convergence gates, and codegraph impact.

## Completed changes

- Removed `RuntimeHandlerSet::fill_missing_from`.
- Removed `ExecutionIndex::handlers_for_ability`.
- Added `ExecutionIndex::unique_handler_slot` and
  `ExecutionIndex::unique_mode_registered`.
- Migrated `has_*` and `resolve_*` projections to fail closed when the
  requested ability-name slot is ambiguous across authority-scoped rows.
- Migrated runtime sync to `runtime_handlers_for_key`, so LocalRuntime
  registration reads handlers from the exact authority-scoped execution key.
- Added regression tests for same-name same-slot ambiguity and cross-authority
  mode synthesis.
- Extended architecture and SPEC v2 gates to reject ability-name handler-set
  merge and missing-slot fallback reintroduction.

## Verification

- `cargo test -q ability_name_handler_projection --lib`
  - 2 passed.
- `cargo test -q hot_register_rpc_is_visible_to_resolve_rpc_and_has_rpc --lib`
  - 1 passed.
- `cargo test -q hot_register_preserves_prior_dynamic_call_modes --lib`
  - 1 passed.
- `cargo test -q dynamic_registration_rollback_restores_prior_snapshot --lib`
  - 1 passed.
- `cargo fmt --check`
  - passed.
- `git diff --check`
  - passed.
- `check-architecture-convergence.sh`
  - passed.
- `check-canonical-runtime-convergence-v2.sh`
  - passed.
- `codegraph callers unique_handler_slot`
  - direct callers are the six `resolve_*` projection methods.
- `codegraph callers runtime_handlers_for_key`
  - direct caller is `sync_runtime_ability`.
- `codegraph query handlers_for_ability`
  - no symbol results.
