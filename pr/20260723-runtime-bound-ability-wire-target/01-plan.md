# Runtime-bound ability wire target convergence

## Goal

Replace raw-string target matching inside `RuntimeBoundAbility` with an explicit
wire-target abstraction so descriptor dispatch does not preserve an implicit
"historic form" fallback path.

## Root abstraction problem

`RuntimeBoundAbility::require_wire_target_matches` accepts a raw `wire_target`
and directly calls `ability_ura_for_wire`. That helper accepts both
owner-local public names and descriptor-bound refs. The public behavior is
intentional, but the runtime-bound ability object should not model it as one
undifferentiated string. The dispatch boundary needs explicit states:
descriptor-bound target vs owner-local target.

## Invariants

1. Descriptor-bound targets remain the canonical signed wire path.
2. Owner-local public names may remain as a public ingress selector, but only as
   an explicit `OwnerLocal` state inside the dispatch boundary.
3. Both target forms must resolve to the selected runtime Ability URA before
   dispatch.
4. Mismatch failures keep the existing dispatch-key mismatch semantics.
5. `signed_descriptor_ref_from_target` remains descriptor-ref only; it must not
   accept owner-local targets.

## Implementation order

1. Add `WireAbilityTarget` with parse/ability accessors in
   `descriptor_binding.rs`.
2. Migrate `require_wire_target_matches` to the typed target.
3. Update comments/tests/gates to reject raw-string fallback language.
4. Verify targeted descriptor-binding tests and convergence gates.

## Verification

- `cargo test -q wire_target_match --lib`
  - 3 passed.
- `cargo test -q signed_descriptor_ref --lib`
  - 5 passed.
- `cargo fmt --check`
  - passed.
- `git diff --check`
  - passed.
- `check-architecture-convergence.sh`
  - passed.
- `check-canonical-runtime-convergence-v2.sh`
  - passed.
- `codegraph sync .`
  - synced 1 changed source file.
- `codegraph status .`
  - index up to date.
- `codegraph callers require_wire_target_matches`
  - production callers are the selected-route unary, stream, and bidi dispatch
    paths.
- `codegraph impact WireAbilityTarget`
  - impact is contained inside `descriptor_binding.rs`.
