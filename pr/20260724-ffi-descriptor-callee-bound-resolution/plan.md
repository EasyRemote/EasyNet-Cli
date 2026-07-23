# FFI descriptor callee-bound resolution convergence

## Goal

Remove the FFI descriptor resolver's local ability parsing path and route all
descriptor-ref resolution requests through the shared callee-bound descriptor
ability resolver.

## Root abstraction problem

`runtime_resolve_descriptor_ref_json` accepted an explicit Ability URA by
calling `AbilitySelector::parse` directly. That bypassed the shared
`descriptor_ref::ability_ura_for_wire(callee, ability)` boundary, so an Ability
URA owned by a different callee could still be treated as a lookup key before
catalog resolution failed later.

Descriptor resolution must not have a second interpretation of Ability URA
ownership at the FFI ingress boundary.

## Architectural decision

The FFI descriptor resolver delegates ability normalization to
`descriptor_ref::ability_ura_for_wire`:

- descriptor refs are normalized and checked against the callee;
- explicit Ability URAs must be owned by the callee;
- bare ability names are projected through the callee owner;
- catalog lookup receives only a callee-bound canonical Ability URA.

## Boundary invariants

1. `runtime_resolve_descriptor_ref_json` must not call
   `AbilitySelector::parse(ability)` directly.
2. `runtime_resolve_descriptor_ref_json` must not call
   `owner_ability_ura(callee_ura, ability)` directly.
3. Owner mismatch must fail as an invalid descriptor request before runtime
   owner or catalog lookup.
4. SPEC v2 must reject reintroducing FFI-local descriptor ability parsing.

## Verification

Completed:

- `cargo fmt --check`
- `cargo test -q --features axon-pb runtime_descriptor_resolver_rejects_ability_owner_mismatch_before_catalog_lookup --lib`
- `cargo test -q --features axon-pb runtime_descriptor_resolver_does_not_remote_probe_realm_catalog_miss --lib`
- `cargo test -q --features axon-pb runtime_descriptor_resolver_prefers_local_catalog_for_runtime_owner --lib`
- `cargo check --features axon-pb`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `tools/scripts/check-architecture-convergence.sh`
- `git diff --check`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`
