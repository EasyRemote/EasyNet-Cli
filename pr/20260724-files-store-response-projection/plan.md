# Files Store Response Projection

## Goal

Move `files.put`, `files.get`, and `files.list` public response ownership out of the content-addressed store handlers and into typed daemon resource projections. Preserve the public wire shape while keeping storage and DTO responsibilities separate.

## Invariants

1. Files-store handlers do not assemble public response objects with raw `json!`.
2. `files.put` response carries `ura`, `sha256`, `size`, `content_type`, and `filename`.
3. `files.get` response carries `bytes_b64`, `content_type`, `filename`, `sha256`, and `size`.
4. `files.list` response carries `items`, each with `sha256`, `size`, `filename`, `content_type`, and `ura`.
5. Blob metadata remains immutable storage metadata; it does not own public response projection.
6. Unknown fields on response DTOs fail closed.
7. No non-URA terminology is introduced.

## Boundary Proof

- `files_store::handlers` owns argument parsing, content-addressed storage, metadata read/write, and selector parsing.
- `daemon::resources::projection` owns public files-store DTOs.
- `files_store::state` owns storage path and blob URA construction.
- No product or SDK-specific file lifecycle is introduced.

## Verification Plan

- Unit tests for typed files-store put/get/list public wire shape.
- Strict unknown-field rejection tests for files-store response DTOs.
- Existing files-store round-trip/idempotency/list/metadata tests.
- SPEC v2 gate coverage preventing handler-owned files-store response JSON assembly from returning.
- Existing SPEC v2, architecture gate, cargo check, rustfmt, diff check, and codegraph sync/status.

## Implementation Delta

- Added typed `FilesPutResponse`, `FilesGetResponse`, `FilesListItem`, and `FilesListResponse` DTOs to `daemon::resources::projection`.
- Migrated `files.put`, `files.get`, and `files.list` handlers to return typed projections through `serde_json::to_value`.
- Removed production `Ok(json!({ ... }))` response assembly from files-store handlers.
- Removed production `items.push(json!({ ... }))` list item assembly from files-store handlers.
- Moved the `json` macro import into tests so production handler imports only `serde_json::Value`.
- Extended SPEC v2 gate coverage and self-test fixtures to reject legacy files-store response assembly.

## Verification Results

- `cargo test -q --features axon-pb files_put_response_preserves_public_shape --lib` — passed.
- `cargo test -q --features axon-pb files_get_response_preserves_public_shape --lib` — passed.
- `cargo test -q --features axon-pb files_list_response_preserves_public_shape --lib` — passed.
- `cargo test -q --features axon-pb files_store_response_dtos_reject_unknown_fields --lib` — passed.
- `cargo test -q --features axon-pb put_then_get_round_trips_bytes --lib` — passed.
- `cargo test -q --features axon-pb list_returns_items_after_put --lib` — passed.
- `cargo test -q --features axon-pb put_rejects_metadata_conflict_for_existing_blob --lib` — passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh` — passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test` — passed.
- `tools/scripts/check-architecture-convergence.sh` — passed.
- `cargo check -q --features axon-pb` — passed.
- `cargo fmt --check` — passed.
- `git diff --check` — passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph status .` — passed with index up to date.
- `/Users/macbook.silan.tech/.local/bin/codegraph affected src/daemon/ability/builtins/resources/files_store/handlers.rs src/daemon/resources/projection.rs tools/scripts/check-canonical-runtime-convergence-v2.sh` — returned broad SDK/runtime candidates through the shared projection/gate graph; focused daemon files-store tests and SPEC gates cover this change.

## Follow-up Seam

Continue moving public response DTOs out of resources handlers. The next compact candidate is `pages/list_get_unpublish.rs`, where project list/get/unpublish responses are still assembled in the handler module.
