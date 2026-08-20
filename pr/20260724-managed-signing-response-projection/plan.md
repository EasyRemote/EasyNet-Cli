# Managed Signing Response Projection

## Goal

Move managed-signing create/list/get-public response ownership out of keyring ability handlers and into a typed keyring projection module. Preserve the public wire shape while keeping provider custody, key creation, inventory filtering, public-key lookup, and fingerprint derivation inside `keyring/abilities.rs`.

## Invariants

1. `keyring.create`, `keyring.list`, and `keyring.get_public` handlers do not assemble their public response payloads with raw response JSON.
2. Create response carries `key_id`, `public_key`, `fingerprint`, and `rotation_epoch`.
3. List response carries `entries`, and each entry omits private seed material and public key bytes.
4. Get-public response carries `public_key`, `fingerprint`, `status`, and `rotation_epoch`.
5. Managed-signing lifecycle and provider calls remain in `keyring/abilities.rs`.
6. Public response DTO ownership sits in `keyring::managed_signing_projection`.
7. Unknown fields on managed-signing response DTOs fail closed.
8. Only URA terminology is used.

## Boundary Proof

- `keyring/abilities.rs` owns request validation, provider interaction, public-key decoding, and fingerprint derivation.
- `keyring::managed_signing_projection` owns the public response DTOs and status string projection.
- `keyring::Vault` remains the encrypted private-key custody and lifecycle state-machine boundary.
- No SDK abstraction, fallback route, or product-specific lifecycle is introduced.

## Verification Plan

- Unit tests for create/list/get-public response public wire shapes.
- Strict unknown-field rejection tests for all managed-signing response DTOs.
- Handler tests proving create/list/get-public return typed public shapes and list does not leak public key bytes or signer policy refs.
- SPEC v2 gate coverage preventing handler-owned managed-signing response JSON assembly from returning.
- Existing SPEC v2, architecture gate, cargo check, rustfmt, diff check, and codegraph sync/status.

## Implementation Delta

- Added `keyring::managed_signing_projection` as the DTO owner for managed-signing create/list/get-public responses.
- Removed the old `entry_view` raw JSON projection helper from `keyring/abilities.rs`.
- Migrated `handle_create`, `handle_list`, and `handle_get_public` to typed DTO constructors.
- Kept provider calls, public-key decoding, and fingerprint derivation inside `keyring/abilities.rs`.
- Added strict DTO coverage that rejects private seed, public key material in list responses, and signer policy refs in public response DTOs.
- Added SPEC v2 gate coverage and self-test fixture to reject handler-owned managed-signing response assembly.

## Verification Results

- `cargo test -q --features axon-pb managed_signing_create_response_preserves_public_shape --lib`
- `cargo test -q --features axon-pb managed_signing_list_response_preserves_public_shape_without_key_material --lib`
- `cargo test -q --features axon-pb managed_signing_public_response_preserves_public_shape --lib`
- `cargo test -q --features axon-pb managed_signing_response_dtos_reject_unknown_fields --lib`
- `cargo test -q --features axon-pb create_then_list_then_get_public --lib`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `tools/scripts/check-architecture-convergence.sh`
- `cargo fmt --check`
- `git diff --check`
- `cargo check -q --features axon-pb`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`
- `/Users/macbook.silan.tech/.local/bin/codegraph affected src/daemon/keyring/abilities.rs src/daemon/keyring/managed_signing_projection.rs src/daemon/keyring/mod.rs tools/scripts/check-canonical-runtime-convergence-v2.sh`

## Follow-up Seam

The remaining keyring managed-signing lifecycle responses (`rotate`, `revoke`, `expire_set`, `bind_subject`, `peer_add`, `peer_list`) still include local raw response assembly. They should be migrated as one or more lifecycle-focused seams because they represent state transitions rather than simple public projection reads.
