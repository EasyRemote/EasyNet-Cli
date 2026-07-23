# Managed Signing Lifecycle Response Projection

## Goal

Remove the remaining handler-owned managed-signing lifecycle and peer response JSON assembly. Keyring handlers should own input validation and provider calls only; `managed_signing_projection` owns the public wire DTOs.

## Invariants

- Provider custody and lifecycle transitions remain in the keyring provider/Vault boundary.
- Public response field names remain compatible:
  - rotate: `new_key_id`, `retired_key_id`, `rotation_epoch`
  - revoke: `tombstone_unix_ms`
  - ack: `ok`
  - peer add: `added`
  - peer list: `peers[]` with `peer_ura`, `fingerprint`, `public_key`, `status`, `via_hub`, `added_unix_ms`, `last_seen_unix_ms`
- Response DTOs use `#[serde(deny_unknown_fields)]` to keep projection fail-closed.
- Handler code must not manually assemble these public JSON objects after migration.
- Peer trust wire vocabulary belongs to the projection layer, not individual handlers.

## Boundary Proof

`ManagedSigningProvider` remains the only abstraction that mutates or queries managed-signing lifecycle state. `managed_signing_projection` depends only on public inventory projections (`ManagedSigningKeyProjection`, `ManagedPeer`, `ManagedSigningStatus`) and cannot sign, persist, rotate, revoke, or mutate custody.

## Verification Plan

- Focused Rust tests for lifecycle/peer DTO shapes and handler round trips.
- SPEC v2 gate coverage for required DTOs, handler constructor usage, and retired raw JSON assembly patterns.
- `cargo fmt --check`, `cargo check --features axon-pb`, `git diff --check`.
- codegraph sync/status/affected after edits.

## Implementation Delta

- Added typed lifecycle/peer DTOs:
  - `ManagedSigningRotateResponse`
  - `ManagedSigningRevokeResponse`
  - `ManagedSigningAckResponse`
  - `ManagedSigningPeerAddResponse`
  - `ManagedSigningPeerListEntry`
  - `ManagedSigningPeerListResponse`
- Migrated rotate/revoke/expire/bind/peer handlers from raw `json!` response construction to projection constructors.
- Moved peer trust wire vocabulary (`trusted`) into the projection layer.
- Extended handler tests to assert response shape and absence of persistence/internal field names.
- Extended SPEC v2 gate and self-test fixture to reject handler-owned lifecycle response assembly.

## Verification Results

- `cargo test -q --features axon-pb managed_signing_rotate_response_preserves_public_shape --lib`
- `cargo test -q --features axon-pb managed_signing_revoke_response_preserves_public_shape --lib`
- `cargo test -q --features axon-pb managed_signing_ack_response_preserves_public_shape --lib`
- `cargo test -q --features axon-pb managed_signing_peer_add_response_preserves_public_shape --lib`
- `cargo test -q --features axon-pb managed_signing_peer_list_response_preserves_public_shape --lib`
- `cargo test -q --features axon-pb managed_signing_lifecycle_response_dtos_reject_unknown_fields --lib`
- `cargo test -q --features axon-pb rotate_then_revoke_round_trip --lib`
- `cargo test -q --features axon-pb peer_add_then_list_round_trip --lib`
- `cargo test -q --features axon-pb expire_set_and_bind_subject_persist --lib`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `tools/scripts/check-architecture-convergence.sh`
- `cargo fmt --check`
- `git diff --check`
- `cargo check -q --features axon-pb`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`
- `/Users/macbook.silan.tech/.local/bin/codegraph affected src/daemon/keyring/abilities.rs src/daemon/keyring/managed_signing_projection.rs tools/scripts/check-canonical-runtime-convergence-v2.sh`

## Follow-up Seam

Managed-signing federated user token handlers still assemble public response JSON. That path mixes token exchange DTO ownership with keyring lifecycle ownership and should be split into a dedicated federation/user-binding projection module in a later iteration.
