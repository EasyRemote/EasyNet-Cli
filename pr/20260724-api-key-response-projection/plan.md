# API Key Response Projection

## Goal

Move governance API key public response ownership out of the bearer-token lifecycle handler and into a typed governance projection module. Preserve the public wire shape while keeping token minting, hashing, store mutation, list filtering, and revoke semantics inside `api_key.rs`.

## Invariants

1. API key handlers do not assemble public create/list/revoke payloads with raw response JSON.
2. Create response returns the raw bearer token only once and never persists it.
3. List response never exposes `token_hash` or raw token material.
4. Revoke response only exposes the revoked `id_prefix`.
5. Public response DTO ownership sits in `governance::api_key_projection`.
6. Unknown fields on API key response DTOs fail closed.
7. Only URA terminology is used.

## Boundary Proof

- `governance/api_key.rs` owns bearer token minting, token hashing, TOML store mutation, user filtering, and revoke lifecycle.
- `governance/api_key_projection.rs` owns the public response DTOs for API key create/list/revoke.
- `api_keys.toml` remains the secret-derived persistence boundary; public list projection copies only operator-safe fields.
- No SDK abstraction, fallback route, product lifecycle, or compatibility layer is introduced.

## Verification Plan

- Unit tests for typed create/list/revoke response public wire shapes.
- Strict unknown-field rejection tests for all API key response DTOs.
- Handler tests proving create/list/revoke return typed public shapes and do not leak `token_hash`.
- SPEC v2 gate coverage preventing handler-owned API key response JSON assembly from returning.
- Existing SPEC v2, architecture gate, cargo check, rustfmt, diff check, and codegraph sync/status.

## Implementation Delta

- Added `governance::api_key_projection` as the public DTO owner for API key create/list/revoke responses.
- Moved raw create/list/revoke response assembly out of `governance/api_key.rs`.
- Kept token generation, hashing, TOML store mutation, list filtering, and revoke lifecycle in `governance/api_key.rs`.
- Added strict DTO coverage that rejects `token_hash`, raw token leakage in list responses, and revoke scope leakage.
- Added SPEC v2 gate coverage and self-test fixture to prevent handler-owned API key response JSON assembly from returning.

## Verification Results

- `cargo test -q --features axon-pb api_key_create_response_preserves_public_shape_without_hash --lib`
- `cargo test -q --features axon-pb api_key_list_response_preserves_public_shape_without_secret_material --lib`
- `cargo test -q --features axon-pb api_key_revoke_response_preserves_public_shape --lib`
- `cargo test -q --features axon-pb api_key_response_dtos_reject_unknown_fields --lib`
- `cargo test -q --features axon-pb create_list_revoke_return_typed_public_shapes_without_secret_leaks --lib`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `tools/scripts/check-architecture-convergence.sh`
- `cargo fmt --check`
- `git diff --check`
- `cargo check -q --features axon-pb`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`
- `/Users/macbook.silan.tech/.local/bin/codegraph affected src/daemon/ability/builtins/governance/api_key.rs src/daemon/ability/builtins/governance/api_key_projection.rs src/daemon/ability/builtins/governance/mod.rs tools/scripts/check-canonical-runtime-convergence-v2.sh`

## Follow-up Seam

Continue the global raw-response scan. Strong candidates are keyring managed-signing public projections and governance access-control responses, but the next seam should be chosen by authority boundary and testability rather than raw match count.
