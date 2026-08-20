# Managed Signing Issuer Provider Narrowing

## Goal

Split the user-binding issuance provider dependency away from the broad managed-signing administration provider. Issuing a federated user-binding token only needs public-key projection and canonical signing. It must not depend on create/list/rotate/revoke/expiry/bind/peer lifecycle operations.

## Invariants

- Public ability request and response behavior remains unchanged.
- `abilities.rs` may keep using the full `ManagedSigningProvider` for management abilities.
- `UserBindingIssueStateMachine` depends only on an issuer provider seam with `public_key` and `sign`.
- The issuer state-machine tests must not implement unreachable administration or peer methods.
- The SPEC v2 gate must reject broad provider coupling inside `user_binding_issue.rs`.

## Boundary Proof

`ManagedSigningProvider` is the administration-capable provider used by the keyring ability adapter. `ManagedSigningIssuerProvider` is the narrow runtime provider required by token issuance state machines. The narrow provider is implemented for any full provider, so existing production wiring stays stable while the state machine dependency becomes semantically minimal.

## Verification Plan

- Focused issue state-machine tests.
- Existing federated token issuance handler tests.
- SPEC v2 gate and self-test.
- Architecture convergence gate.
- `cargo check`, fmt, diff check, and codegraph sync/status.

## Implementation Delta

- Added `ManagedSigningIssuerProvider` with only `public_key` and `sign`.
- Implemented the issuer seam for every full `ManagedSigningProvider`, preserving production wiring without making issue state machines depend on administration methods.
- Made `UserBindingIssueStateMachine` generic over the narrow issuer provider.
- Removed unreachable create/list/rotate/revoke/expiry/bind/peer implementations from the issue state-machine test provider.
- Extended SPEC v2 so `user_binding_issue.rs` cannot import the broad provider or reintroduce unreachable administration methods in its tests.
- Updated the SPEC v2 self-test fixture so it satisfies the narrow provider shape and still fails only when inline handler lifecycle logic returns.

## Verification Results

- `cargo check -q --features axon-pb`
- `cargo test -q --features axon-pb issue_state_machine_returns_signed_token --lib`
- `cargo test -q --features axon-pb issue_state_machine_rejects_self_target_realm --lib`
- `cargo test -q --features axon-pb issue_state_machine_rejects_unbound_signing_key --lib`
- `cargo test -q --features axon-pb federate_user_identity_token_happy_path --lib`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `tools/scripts/check-architecture-convergence.sh`
- `cargo fmt --check`
- `git diff --check`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
- `/Users/macbook.silan.tech/.local/bin/codegraph affected src/daemon/keyring/managed_signing_provider.rs src/daemon/keyring/user_binding_issue.rs tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`

## Follow-up Seam

The remaining broad `ManagedSigningProvider` is still appropriate for the management ability adapter, but it should not become the default dependency for future state machines. Future state-machine extractions should introduce capability-specific provider seams before wiring handlers.
