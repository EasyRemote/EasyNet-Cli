# Pages Management Response Projection

## Goal

Move `pages.project_list`, `pages.get`, and `pages.unpublish` public response ownership out of the Pages management handlers and into typed daemon resource projections. Preserve the public wire shape while separating registry mutation/read logic from DTO construction.

## Invariants

1. Pages list/get/unpublish handlers do not assemble public response objects with raw `json!`.
2. `pages.project_list` response carries `projects`.
3. Each list item carries `user`, `project_id`, `folder`, `visibility`, `started_at_ms`, `url_root`, and `dev_listener_url_root`.
4. `pages.get` response carries the existing detail fields, including `project_ura` and `file_size_cap`.
5. `pages.unpublish` response carries `user`, `project_id`, and `removed`.
6. Registry ownership remains in `pages::state`; projection ownership sits in `daemon::resources::projection`.
7. `pages.health` remains out of scope for this iteration.
8. Unknown fields on response DTOs fail closed.
9. Only URA terminology is used.

## Boundary Proof

- `pages::state` owns process-local published project handles and persistence snapshots.
- `pages/list_get_unpublish.rs` owns ability ingress, project lookup/removal, hot ability unregistering, and registry persistence.
- `daemon::resources::projection` owns public Pages management DTOs.
- Public URL construction stays in `pages` module helpers because those helpers are the canonical Pages URL policy.

## Verification Plan

- Unit tests for typed Pages list/get/unpublish public wire shapes.
- Strict unknown-field rejection tests for Pages response DTOs.
- Existing Pages health tests remain unchanged.
- Add focused handler tests for list/get/unpublish response shapes.
- SPEC v2 gate coverage preventing handler-owned Pages management response JSON assembly from returning.
- Existing SPEC v2, architecture gate, cargo check, rustfmt, diff check, and codegraph sync/status.

## Implementation Delta

- Added typed `PagesProjectListItem`, `PagesProjectListResponse`, `PagesProjectDetailResponse`, and `PagesUnpublishResponse` DTOs to `daemon::resources::projection`.
- Migrated `pages.project_list`, `pages.get`, and `pages.unpublish` handlers to return typed projections through `serde_json::to_value`.
- Removed handler-owned list-entry raw JSON assembly for `pages.project_list`.
- Removed handler-owned detail response raw JSON assembly for `pages.get`.
- Removed handler-owned unpublish receipt raw JSON assembly for `pages.unpublish`.
- Added shared helper projection functions in the handler module so list/get no longer duplicate started-at and URL field assembly.
- Extended SPEC v2 gate coverage and self-test fixtures to reject legacy Pages management response assembly.

## Verification Results

- `cargo test -q --features axon-pb pages_project_list_response_preserves_public_shape --lib` — passed.
- `cargo test -q --features axon-pb pages_project_detail_response_preserves_public_shape --lib` — passed.
- `cargo test -q --features axon-pb pages_unpublish_response_preserves_public_shape --lib` — passed.
- `cargo test -q --features axon-pb pages_management_response_dtos_reject_unknown_fields --lib` — passed.
- `cargo test -q --features axon-pb handle_list_returns_typed_project_projection_shape --lib` — passed.
- `cargo test -q --features axon-pb handle_get_returns_typed_project_detail_shape --lib` — passed.
- `cargo test -q --features axon-pb handle_unpublish_returns_typed_receipt_and_removes_project --lib` — passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh` — passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test` — passed.
- `tools/scripts/check-architecture-convergence.sh` — passed.
- `cargo check -q --features axon-pb` — passed.
- `cargo fmt --check` — passed.
- `git diff --check` — passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph status .` — passed with index up to date.
- `/Users/macbook.silan.tech/.local/bin/codegraph affected src/daemon/ability/builtins/resources/pages/list_get_unpublish.rs src/daemon/resources/projection.rs tools/scripts/check-canonical-runtime-convergence-v2.sh` — returned broad SDK/runtime candidates through the shared projection/gate graph; focused Pages handler tests and SPEC gates cover this change.

## Follow-up Seam

`pages.health` still owns a readiness DTO in `pages/list_get_unpublish.rs`. It should be migrated separately into a typed readiness projection so the health lifecycle shape can be reviewed without coupling it to list/get/unpublish management responses.
