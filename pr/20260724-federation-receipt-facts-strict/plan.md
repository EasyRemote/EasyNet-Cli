# Federation receipt facts strictness

## Goal

Remove executable compatibility where federation join/heartbeat receipts omit hub ability catalog and advertise-policy facts but the client silently synthesizes defaults.

## Root abstraction problem

The hub ability catalog is a runtime routing fact, not an optional UI enrichment. Treating absent `hub_published_abilities`, `hub_abilities_revision`, `advertise_contract`, or `hub_abilities_diff` as defaults lets a device proceed with an invented route/policy view. That preserves old hub behavior at the exact boundary where the runtime must prove what the hub published.

## Boundary invariants

1. `JoinReceipt` requires explicit `hub_published_abilities`, `hub_abilities_revision`, and `advertise_contract`.
2. `AdvertiseContract` requires explicit `allowed_owner_prefixes` and `allows_hosted_agents`.
3. `HeartbeatReceipt` requires explicit `hub_abilities_diff`.
4. `HubAbilitiesDiff` requires explicit `revision`, `added`, and `removed`.
5. The in-repo hub wrapper emits the same required receipt facts that the client requires.
6. SPEC v2 rejects reintroduced serde defaults and old-hub compatibility language for these federation receipt facts.

## Verification plan

- Run targeted federation ability-contract and session prelude/heartbeat tests.
- Run `cargo fmt --check` and `git diff --check`.
- Run `tools/scripts/check-canonical-runtime-convergence-v2.sh`.
- Run `tools/scripts/check-architecture-convergence.sh`.

## Decisions

- Do not preserve old-hub parse defaults. A hub that omits route/policy facts is not producing the current canonical runtime receipt.
- Keep empty catalog/diff as valid only when represented explicitly by the hub with a revision and empty arrays.
- Move shared federation receipt fact DTOs out of the client-only module into `daemon::federation::receipt_contract`, then re-export them for existing call sites. This prevents hub producer and device consumer DTO drift without changing the public internal path used by existing code.
- The current in-repo hub wrapper has no injected hub-published ability store, so it emits an explicit empty snapshot/diff at revision `0`. This is a producer fact, not a consumer fallback.

## Implementation delta

- Added `src/daemon/federation/receipt_contract.rs` for `JoinReceipt`, `HubAbilityEntry`, `AdvertiseContract`, and `HubAbilitiesDiff`.
- Removed serde defaults and `Default`-based compatibility from join/heartbeat hub catalog facts.
- Updated the federation wrapper to emit explicit join snapshot/policy facts and heartbeat diff facts.
- Updated consumer and producer tests to reject missing runtime facts and accept explicit empty facts.
- Added `check_federation_receipt_facts_strict_contract` to SPEC v2.

## Verification results

- `cargo test -q daemon::federation::client::ability_contract --features axon-pb` passed.
- `cargo test -q daemon::invocation::dispatch::federation_wrappers --features axon-pb` passed.
- `cargo test -q federation_join_receipt_seeds_canonical_hub_catalog --features axon-pb` passed.
- `cargo test -q federation_heartbeat_receipt_applies_revision_only_diff --features axon-pb` passed.
- `cargo fmt --check` passed.
- `git diff --check` passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh` passed.
- `tools/scripts/check-architecture-convergence.sh` passed.
