# Federation revoke ingress strictness

## Goal

Remove the `federation.revoke` public ingress compatibility alias that accepts both `target_ura` and `agent_ura`, and fail closed when the canonical revoke target is absent or non-canonical.

## Root abstraction problem

`federation.revoke` is a membership/directory mutation. Letting request decoding choose between two target fields makes the mutation boundary ambiguous and preserves historical tuple shapes. An empty target also produced a successful no-op path instead of a typed boundary failure.

## Boundary invariants

1. `RevokeRequest` exposes exactly one target identity field: `agent_ura`.
2. Unknown request fields are rejected by serde, including retired `target_ura`.
3. `agent_ura` is required and must parse as a canonical URA before any registry/catalog/persistence mutation.
4. Tests pin canonical `agent_ura` success and retired `target_ura` rejection.
5. SPEC v2 rejects reintroduced `target_ura` aliasing or effective-target fallback logic in `federation.revoke`.

## Verification plan

- Run targeted federation wrapper tests.
- Run targeted unary dispatcher revoke test.
- Run `cargo fmt --check` and `git diff --check`.
- Run `tools/scripts/check-canonical-runtime-convergence-v2.sh`.
- Run `tools/scripts/check-architecture-convergence.sh`.

## Decisions

- Do not keep the alias as a compatibility layer. Current canonical CLI helper already sends `agent_ura`.
- Validate target identity at the request object boundary instead of relying on downstream registry/cache misses.

## Implementation delta

- Added `#[serde(deny_unknown_fields)]` to `RevokeRequest`.
- Removed the retired `target_ura` request field and `effective_target_ura` fallback selector.
- Added `RevokeRequest::canonical_target_ura` to require a non-empty canonical `agent_ura` before mutation.
- Updated dispatcher and wrapper tests to use `agent_ura`.
- Added tests for retired `target_ura` rejection and invalid/missing `agent_ura` rejection.
- Added `check_federation_revoke_ingress_strict_contract` to SPEC v2.

## Verification results

- `cargo test -q daemon::invocation::dispatch::federation_wrappers --features axon-pb` passed.
- `cargo test -q invoke_dispatches_federation_revoke --features axon-pb` passed.
- `cargo fmt --check` passed.
- `git diff --check` passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh` passed.
- `tools/scripts/check-architecture-convergence.sh` passed.
