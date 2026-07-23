# Pages Publish Response Projection

## Goal

Move `pages.publish` public response ownership out of the publish lifecycle handler and into typed daemon resource projections. Preserve the public wire shape while keeping create-transition state mutation, registry persistence, and ability registration inside the publish handler.

## Invariants

1. `pages.publish` handler does not assemble the public publish payload with raw response JSON.
2. Publish response carries `project_ura`, `url_root`, `user`, `project_id`, and `visibility`.
3. Directory validation, sandbox root opening, duplicate rejection, registry persistence, and project ability registration remain in `pages/publish.rs`.
4. Public response DTO ownership sits in `daemon::resources::projection`.
5. Unknown fields on publish response DTOs fail closed.
6. No product-specific SDK abstraction, route fallback, or compatibility layer is introduced.
7. Only URA terminology is used.

## Boundary Proof

- `pages/publish.rs` owns the create transition for a static Pages project.
- `daemon::resources::projection` owns the public publish response shape.
- `pages::state` remains the persistence and in-memory project registry boundary.
- `pages::sandbox` remains the kernel/path safety boundary.

## Verification Plan

- Unit tests for typed `PagesPublishResponse` public wire shape.
- Strict unknown-field rejection test for the publish response DTO.
- Focused handler test proving `handle_publish` returns the same public shape after migration.
- SPEC v2 gate coverage preventing handler-owned Pages publish response JSON assembly from returning.
- Existing SPEC v2, architecture gate, cargo check, rustfmt, diff check, and codegraph sync/status.

## Implementation Delta

- Added `PagesPublishResponse` to `daemon::resources::projection` with strict unknown-field rejection.
- Moved public `pages.publish` payload construction out of `pages/publish.rs` and into the projection DTO.
- Kept directory validation, sandbox root opening, duplicate rejection, registry persistence, and project ability registration in the publish handler.
- Added SPEC v2 gate coverage and self-test fixture to reject handler-owned publish response assembly.
- Added focused handler coverage proving the public wire shape is preserved without leaking local folder or canonical root state.

## Verification Results

- `cargo test -q --features axon-pb pages_publish_response_preserves_public_shape --lib`
- `cargo test -q --features axon-pb pages_publish_response_rejects_unknown_fields --lib`
- `cargo test -q --features axon-pb handle_publish_returns_typed_payload_projection_shape --lib`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `tools/scripts/check-architecture-convergence.sh`
- `cargo fmt --check`
- `git diff --check`
- `cargo check -q --features axon-pb`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`
- `/Users/macbook.silan.tech/.local/bin/codegraph affected src/daemon/ability/builtins/resources/pages/publish.rs src/daemon/resources/projection.rs tools/scripts/check-canonical-runtime-convergence-v2.sh`

## Follow-up Seam

`pages/api.rs` still has local response assembly for project file API operations. That path should be migrated after separating API operation DTOs from project lifecycle DTOs, because file API responses belong to a different public resource projection family than publish/fetch/list/health.
