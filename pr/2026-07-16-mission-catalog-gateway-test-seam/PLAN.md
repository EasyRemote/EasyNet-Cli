# Mission Catalog Gateway Test-Seam Gate

## Goal

Prevent the old Mission direct-catalog invocation bypass from re-entering
production after Mission child dispatch has moved onto Axon child invocation.

## Root Fork

Mission orchestration previously had two possible child-call owners:

- production Axon parent capability via `DaemonMissionInvocationGateway`
- direct `AxonAbilityCatalog` handler dispatch via `CatalogMissionInvocationGateway`

The production owner is now the Axon child invocation path. The catalog gateway
is only valid as a test seam for unit-level orchestration tests.

## Expected Effect

- Architecture convergence: one production child-invocation owner.
- Product acceleration: future Mission work can rely on receipt-bound child
  causal context instead of checking for catalog bypasses manually.
- Cleanliness: regression is blocked by the existing convergence script, not by
  reviewer memory.

## Invariants

1. Production Mission/EAL paths must not call catalog handlers directly.
2. `CatalogMissionInvocationGateway` may exist only behind `#[cfg(test)]`.
3. Production `DaemonMissionInvocationGateway` must derive child dispatch from
   an admitted `AbilityContext`.
4. The regression gate must scan production-stripped Rust source, so test-only
   helper code does not create false positives.

## Verification

- Run `tools/scripts/check-architecture-convergence.sh`.
- Run a focused Mission gateway test to prove the child path still executes.
- Run `git diff --check`.
