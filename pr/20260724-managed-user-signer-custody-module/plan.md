# Managed user signer custody module convergence

## Goal

Separate managed User caller signing custody from runtime-owner self-identity
custody so the daemon identity layer keeps one explicit boundary for User
callers and one explicit boundary for Device/Hub/Agent runtime owners.

## Root abstraction problem

`self_identity.rs` owned too many lifecycle responsibilities:

- daemon runtime-owner signing identity loading;
- runtime caller custody classification;
- managed User signing inventory lookup;
- managed User projection validation;
- managed User signer implementation;
- key-service transport operations.

This made the managed User caller model look like a local detail inside the
runtime-owner self-identity client even though it is a distinct custody model.
That is exactly the seam involved when product calls use a User URA as caller:
User callers must resolve through subject-bound managed signing inventory, not
the singleton runtime-owner key lookup.

## Architectural decision

Create `src/daemon/identity/self_identity/managed_user_signing.rs` as the
managed User signing custody module.

`self_identity.rs` continues to expose the existing public functions and
constants:

- `load_runtime_caller_signer`
- `ensure_user_runtime_signing_identity`
- `EnsuredUserRuntimeSigningIdentity`
- `USER_SIGNING_CLI_PURPOSE`

Internally, `RuntimeCallerSignerResolver` delegates User callers to
`ManagedRuntimeSigningIdentity` in the managed-user module. Runtime-owner
signing remains in `RuntimeSigningIdentity`.

## Boundary invariants

1. `RuntimeSigningIdentity` must reject User URAs before keyring lookup.
2. Managed User signer lookup, projection validation, and signer implementation
   must live in `managed_user_signing.rs`.
3. `self_identity.rs` must not re-own `ManagedRuntimeSigningIdentity` or active
   managed-user lookup logic.
4. Public behavior remains unchanged: User callers use managed signing
   custody; Device/Hub/Agent callers use runtime-owner custody.

## Verification

Completed:

- `cargo fmt --check`
- `cargo check --features axon-pb`
- `cargo test -q --features axon-pb runtime_caller_custody_classifies_user_as_managed_identity --lib`
- `cargo test -q --features axon-pb managed_user_runtime_signer_signs_with_subject_bound_inventory_key --lib`
- `cargo test -q --features axon-pb runtime_caller_signer_resolver_does_not_fall_back_from_user_to_owner_key --lib`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `tools/scripts/check-architecture-convergence.sh`
- `git diff --check`
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
