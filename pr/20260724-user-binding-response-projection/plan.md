# User Binding Response Projection

## Goal

Remove handler-owned public JSON response assembly from the federated user-binding token exchange. The keyring ability handlers should validate inputs, run the token/signature/replay state machine, and call stores/providers; typed projection DTOs should own the public response shapes.

## Invariants

- Issuance response remains wire-compatible:
  - `token`
  - `transport_hint`
- Consume response remains wire-compatible:
  - `binding_recorded`
  - `source_realm`
  - `source_user_ura`
  - `local_user_id`
- Response DTOs use `#[serde(deny_unknown_fields)]`.
- Token signing, signature verification, freshness checks, and replay persistence remain in the existing keyring/user-binding state-machine boundary.
- No EasyNet/EasyRemote SDK abstractions are introduced; this is daemon runtime DTO ownership only.
- No legacy response fallback path remains in the handlers.

## Boundary Proof

`user_binding_projection` depends on public user-binding domain objects (`UserBindingToken`, `FederatedUserBinding`) and owns only serializable response DTOs. It cannot mint signatures, verify signatures, mutate replay stores, or manage key custody. `abilities.rs` remains the orchestrator for the user-binding state machine but no longer owns the response schema.

## Verification Plan

- Focused Rust tests for issue/consume DTO shapes and strict serde rejection.
- Handler tests asserting wire compatibility and absence of request/internal echo fields.
- SPEC v2 gate and self-test fixture rejecting direct handler-owned token response JSON.
- `cargo fmt --check`, `git diff --check`, `cargo check --features axon-pb`, and codegraph sync/status/affected.

## Implementation Delta

- Added `keyring::user_binding_projection`.
- Added fail-closed DTOs:
  - `UserBindingIssueResponse`
  - `UserBindingConsumeResponse`
- Moved `jwt-custom-claim` transport hint vocabulary into the user-binding projection layer.
- Migrated `handle_federate_user_identity_token` and `handle_consume_federate_user_token` away from ad hoc public JSON response assembly.
- Added handler assertions that public responses do not leak request echo fields or persistence fields.
- Extended SPEC v2 gate and self-test coverage for this projection boundary.

## Verification Results

- `cargo test -q --features axon-pb user_binding_issue_response_preserves_public_shape --lib`
- `cargo test -q --features axon-pb user_binding_consume_response_preserves_public_shape --lib`
- `cargo test -q --features axon-pb user_binding_response_dtos_reject_unknown_fields --lib`
- `cargo test -q --features axon-pb federate_user_identity_token_happy_path --lib`
- `cargo test -q --features axon-pb consume_federate_user_token_happy_path --lib`
- `cargo test -q --features axon-pb consume_federate_user_token_full_round_trip_realm_a_to_realm_b --lib`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `tools/scripts/check-architecture-convergence.sh`
- `cargo fmt --check`
- `git diff --check`
- `cargo check -q --features axon-pb`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`
- `/Users/macbook.silan.tech/.local/bin/codegraph affected src/daemon/keyring/abilities.rs src/daemon/keyring/user_binding_projection.rs src/daemon/keyring/mod.rs tools/scripts/check-canonical-runtime-convergence-v2.sh`

## Follow-up Seam

The user-binding token itself is still a direct serde domain object. A later pass should decide whether token wire projection and cryptographic domain representation should remain one type or split once external non-Rust SDK consumers become provider-backed for this path.
