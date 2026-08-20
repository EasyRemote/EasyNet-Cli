# User Binding Issue State Machine Extraction

## Goal

Move federated user-binding token issuance out of `abilities.rs` into an explicit issuer state machine. The handler should parse the public request and serialize the response; the issuer module should own managed-signing precondition checks, source realm derivation, nonce generation, canonical bytes construction, and signature stamping.

## Invariants

- Public request and response behavior remains unchanged.
- Existing error strings remain compatible.
- Managed-signing provider boundary is not owned by the ability handler module.
- Issue transition order remains:
  1. validate target realm
  2. load managed signing public projection
  3. validate active `agent_signing` binding to source user URA
  4. derive source realm and reject self-target
  5. decode source public key
  6. generate nonce
  7. build canonical token bytes
  8. sign and stamp token
- `abilities.rs` must not directly construct or sign `UserBindingToken`.

## Boundary Proof

`managed_signing_provider` owns the provider trait boundary. `user_binding_issue` depends on that provider boundary and the token domain, but not on ability registration or response serialization. `abilities.rs` remains an adapter layer.

## Verification Plan

- Focused issuer state-machine tests for happy path, self-target rejection, and unbound signing key rejection.
- Existing handler issuance tests continue to pass.
- SPEC v2 gate rejects provider trait ownership in `abilities.rs` and inline issue lifecycle logic.
- Run canonical runtime gates, fmt, diff check, cargo check, and codegraph sync/status/affected.

## Implementation Delta

- Added `src/daemon/keyring/managed_signing_provider.rs` as the managed-signing provider boundary.
- Re-exported `ManagedSigningProvider` from `abilities.rs` to preserve existing public Rust call sites while moving ownership out of the handler module.
- Added `src/daemon/keyring/user_binding_issue.rs` with `UserBindingIssueRequest` and `UserBindingIssueStateMachine`.
- Moved target-realm validation, managed-signing public projection loading, active `agent_signing` binding checks, source realm derivation, public-key decode, nonce generation, canonical bytes construction, and signature stamping out of `abilities.rs`.
- Updated SPEC v2 gates so issuance must consume `core::ura::user_realm_from_ura` from the issue state machine rather than from the ability adapter.
- Added SPEC v2 self-test coverage that rejects reintroducing inline issue lifecycle logic in `abilities.rs`.

## Verification Results

- `cargo check -q --features axon-pb`
- `cargo test -q --features axon-pb issue_state_machine_returns_signed_token --lib`
- `cargo test -q --features axon-pb issue_state_machine_rejects_self_target_realm --lib`
- `cargo test -q --features axon-pb issue_state_machine_rejects_unbound_signing_key --lib`
- `cargo test -q --features axon-pb federate_user_identity_token_happy_path --lib`
- `cargo test -q --features axon-pb federate_user_identity_token_two_calls_have_distinct_nonces --lib`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `tools/scripts/check-architecture-convergence.sh`
- `git diff --check`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`

## Follow-up Seam

The provider trait still spans inventory administration, signing, and peer trust operations. This slice moved ownership out of the ability adapter without changing its public shape. A future cohesive refactor should split narrow read/sign/admin peer provider interfaces once all state machines have migrated off the broad provider trait.
