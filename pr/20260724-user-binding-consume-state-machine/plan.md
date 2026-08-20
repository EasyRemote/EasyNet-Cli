# User Binding Consume State Machine Extraction

## Goal

Move the federated user-binding consume lifecycle out of the keyring ability handler and into an explicit state-machine module. The handler should parse the public request and return the public response; the consume module should own target-realm admission, freshness checks, signature verification, replay detection, and binding persistence.

## Invariants

- Public request and response behavior remains unchanged.
- Check order remains unchanged:
  1. target realm
  2. freshness / future-date
  3. signature verification
  4. replay detection
  5. atomic binding + nonce record
- Duplicate nonce races remain guarded by `FederatedBindingsStore::record_binding`.
- Error messages remain compatible with existing tests and operator expectations.
- `abilities.rs` must not directly call `verify_user_binding_signature`, `nonce_seen`, or `record_binding` for consume.

## Boundary Proof

`user_binding_consume` depends on the token domain and binding store, but not on ability catalog registration, keyring provider custody, or response DTO serialization. `abilities.rs` remains an adapter layer only: JSON request projection in, typed state-machine execution, typed response projection out.

## Verification Plan

- Focused unit tests for consume state-machine happy path, wrong target, and replay rejection.
- Existing handler consume tests continue to pass.
- SPEC v2 gate rejects legacy inline consume lifecycle logic in `abilities.rs`.
- Run canonical runtime gates, fmt, diff check, cargo check, and codegraph sync/status/affected.

## Implementation Delta

- Added `keyring::user_binding_consume`.
- Added `UserBindingConsumeRequest` as the typed consume request projection.
- Added `UserBindingConsumeStateMachine` with explicit ordered transitions:
  - `ensure_target_realm`
  - `ensure_freshness`
  - `ensure_signature`
  - `ensure_not_replayed`
  - `record_binding`
- Migrated `handle_consume_federate_user_token` to call the state machine and use `UserBindingConsumeResponse` for serialization.
- Removed direct consume calls to `verify_user_binding_signature`, `nonce_seen`, `record_binding`, and `FederatedUserBinding` construction from the ability handler.
- Extended SPEC v2 gate and self-test coverage to reject inline consume lifecycle logic in `abilities.rs`.

## Verification Results

- `cargo test -q --features axon-pb consume_state_machine_records_binding_after_all_checks --lib`
- `cargo test -q --features axon-pb consume_state_machine_rejects_wrong_target_before_recording --lib`
- `cargo test -q --features axon-pb consume_state_machine_rejects_replay_before_second_recording --lib`
- `cargo test -q --features axon-pb consume_federate_user_token_happy_path --lib`
- `cargo test -q --features axon-pb consume_federate_user_token_rejects_replay --lib`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `tools/scripts/check-architecture-convergence.sh`
- `cargo fmt --check`
- `git diff --check`
- `cargo check -q --features axon-pb`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`
- `/Users/macbook.silan.tech/.local/bin/codegraph affected src/daemon/keyring/abilities.rs src/daemon/keyring/user_binding_consume.rs src/daemon/keyring/mod.rs tools/scripts/check-canonical-runtime-convergence-v2.sh`

## Follow-up Seam

The token issuance path still performs signing orchestration in `abilities.rs`. That path can be reviewed next for a symmetrical issuer state machine if the signing preconditions and nonce generation need the same lifecycle treatment.
