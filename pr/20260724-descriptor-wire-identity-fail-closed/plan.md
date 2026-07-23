# Descriptor wire identity fail-closed convergence

## Goal

Remove the duplicated serialize-time ability URA fallback in the daemon
ability descriptor wire projection.

## Root abstraction problem

`AbilityDescriptorWire::try_from_descriptor` projected the canonical
`ability_ura` through two paths:

- direct `owner_ura + public_name` construction;
- fallback to `AbilityDescriptor::canonical_ability_ura()`.

Those paths represent the same canonical identity source. Keeping both makes a
broken descriptor aggregate look recoverable at the projection boundary, which
is the wrong behavior for descriptor routing: malformed identity state must fail
closed before it can enter catalog, descriptor-ref, or route projections.

## Architectural decision

Serialize-time descriptor identity projection has one source of truth:
`owner_ura + public_name`. If those fields do not derive a canonical Ability
URA, serialization fails. Deserialization still validates supplied wire identity
facts against the recomputed canonical aggregate.

## Boundary invariants

1. Ability descriptor wire projection must not fallback from one canonical URA
   derivation helper to another.
2. Descriptor identity corruption after construction must fail at serialization.
3. Existing public wire fields remain unchanged when descriptors are valid.
4. SPEC v2 must reject reintroduction of the duplicate fallback.

## Verification

Completed:

- `cargo fmt --check`
- `cargo test -q --features axon-pb descriptor_wire_projection_fails_closed_for_corrupt_identity --lib`
- `cargo test -q --features axon-pb descriptor_wire_exposes_canonical_descriptor_ref --lib`
- `cargo check --features axon-pb`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `tools/scripts/check-architecture-convergence.sh`
- `git diff --check`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`
