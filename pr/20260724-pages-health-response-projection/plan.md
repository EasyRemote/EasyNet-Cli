# Pages Health Response Projection

## Goal

Move `pages.health` readiness response ownership out of the Pages management handler and into typed daemon resource projections. Preserve the public wire shape while separating readiness DTO construction from registry inspection.

## Invariants

1. `pages.health` handler does not assemble the public readiness envelope with raw JSON.
2. The response carries `state`, `ready`, `owner_ura`, `surface_ref`, `page_count`, and `checks`.
3. Each check carries `name`, `state`, `ready`, `message`, `latency_ms`, and `metadata`.
4. Registry inspection remains in `pages/list_get_unpublish.rs`.
5. Public readiness DTO ownership sits in `daemon::resources::projection`.
6. Unknown fields on readiness DTOs fail closed.
7. Only URA terminology is used.

## Boundary Proof

- `pages/list_get_unpublish.rs` owns `pages.health` ability ingress, optional target parsing, registry counting, and project existence decision.
- `daemon::resources::projection` owns the public readiness DTOs and canonical check envelope shape.
- `pages::state` continues to own the process-local published project registry.

## Verification Plan

- Unit tests for typed Pages health response and check public wire shape.
- Strict unknown-field rejection tests for health response DTOs.
- Existing aggregate/missing/foreign `pages.health` handler tests.
- Add focused handler test for typed project-present readiness shape.
- SPEC v2 gate coverage preventing handler-owned Pages health response JSON assembly from returning.
- Existing SPEC v2, architecture gate, cargo check, rustfmt, diff check, and codegraph sync/status.

## Implementation Delta

- Added typed `PagesHealthCheck` and `PagesHealthResponse` DTOs to `daemon::resources::projection`.
- Moved the `pages_registry` and `project` check constructors into the projection boundary.
- Migrated `pages.health` to return `PagesHealthResponse` through `serde_json::to_value`.
- Removed production raw readiness envelope construction from `pages/list_get_unpublish.rs`.
- Removed production `json` macro import from `pages/list_get_unpublish.rs`.
- Added focused project-present handler coverage for the ready readiness path.
- Extended SPEC v2 gate coverage and self-test fixtures to reject legacy Pages health response assembly.

## Verification Results

- `cargo test -q --features axon-pb pages_health_response_preserves_public_shape --lib` — passed.
- `cargo test -q --features axon-pb pages_health_response_dtos_reject_unknown_fields --lib` — passed.
- `cargo test -q --features axon-pb handle_health_reports_aggregate_ready_without_projects --lib` — passed.
- `cargo test -q --features axon-pb handle_health_reports_missing_project_as_degraded --lib` — passed.
- `cargo test -q --features axon-pb handle_health_reports_project_present_as_ready_projection --lib` — passed.
- `cargo test -q --features axon-pb handle_health_rejects_foreign_surface_ref --lib` — passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh` — passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test` — passed.
- `tools/scripts/check-architecture-convergence.sh` — passed.
- `cargo check -q --features axon-pb` — passed.
- `cargo fmt --check` — passed.
- `git diff --check` — passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph status .` — passed with index up to date.
- `/Users/macbook.silan.tech/.local/bin/codegraph affected src/daemon/ability/builtins/resources/pages/list_get_unpublish.rs src/daemon/resources/projection.rs tools/scripts/check-canonical-runtime-convergence-v2.sh` — returned broad SDK/runtime candidates through the shared projection/gate graph; focused Pages health tests and SPEC gates cover this change.

## Follow-up Seam

Continue through `ability/builtins/resources/*` for remaining raw public DTO assembly. Compact candidates include `pages/fetch.rs`, `pages/publish.rs`, and `context/ability.rs`.
