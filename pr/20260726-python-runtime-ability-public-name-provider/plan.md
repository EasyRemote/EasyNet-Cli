# Python runtime ability public-name provider cutover

## Goal

Remove the Python SDK runtime ability fallback that derives a scope candidate by
splitting Ability URA text at `/ability/`. Runtime ability scope admission must
use the canonical Addressing provider's projected public ability name.

## Root abstraction problem

`_RuntimeAbilityProjection.from_descriptor_ref` already resolves descriptor
identity through `AddressingClient.project_descriptor_ref` and
`AddressingClient.project_ability_ura`, but it still computes `wire` via
`_descriptor_wire_ability`. That helper duplicates Axon path grammar in the SDK
facade and preserves a legacy fallback if provider public-name projection were
missing.

## Invariants

1. Descriptor references are still validated by the Addressing provider.
2. Public-name scope admission is only available when the descriptor owner
   matches the call callee.
3. The public name comes from `AddressingProjection.public_name`, not URA path
   string splitting.
4. `ability_ura` remains a scope candidate for explicit URA scopes.
5. No `/ability/` grammar helper remains in Python runtime ability lowering.

## Boundary proof

The SDK runtime ability layer lowers addressed capability calls into complete
Invocation drafts. It may consume provider-projected identity facts, but it must
not own protocol grammar. Addressing owns descriptor and Ability URA projection;
runtime ability lowering owns policy composition and authority validation.

## Verification plan

- Python runtime ability targeted tests.
- Python bytecode compile for changed Python source.
- canonical runtime convergence v2 gate and self-test.
- SDK canonical public API attestation if source hashes change.
- `git diff --check`.

