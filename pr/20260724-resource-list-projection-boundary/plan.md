# Resource List Projection Boundary

## Goal

Move `meta.list_resources` response ownership out of the ability handler and into a typed daemon resource projection. Preserve the public wire shape while removing handler-level raw JSON resource entry assembly.

## Invariants

1. `meta.list_resources` returns a typed projection response, not handler-owned raw JSON.
2. Each public resource entry carries only `resource_ura`, `owner_agent`, `type`, `binding`, `display_name`, and `metadata`.
3. Persistence-only fields such as `hardware_id` and `first_seen_at` remain file-local.
4. Unknown fields on resource list DTOs fail closed.
5. Ability handler responsibility is limited to argument parsing, loading resources, filtering, and invoking projection constructors.
6. No URI terminology is introduced.

## Boundary Proof

- `daemon::persistence::resources` owns on-disk resource records and validation.
- `daemon::resources::projection` owns public resource-list DTOs.
- `daemon::ability::builtins::resources::list` owns the `meta.list_resources` ability ingress and no longer hand-assembles public resource entries.

## Verification Plan

- Unit tests for resource-list entry and response public wire shape.
- Strict unknown-field rejection tests for entry and response DTOs.
- Existing `meta.list_resources` handler shape test.
- SPEC v2 gate coverage preventing handler-owned raw resource list response assembly from returning.
- Existing SPEC v2, architecture gate, cargo check, rustfmt, diff check, and codegraph sync/status.

## Implementation Delta

- Added `daemon::resources::projection` as the public resource discovery DTO boundary.
- Added typed `ResourceListEntry` and `ResourceListResponse` projections with strict serde boundaries.
- Migrated `meta.list_resources` from handler-owned raw resource entry JSON to `ResourceListResponse::from_entries`.
- Removed the handler-local `project` function and the raw `{ "resources": wire }` envelope assembly.
- Kept persistence-only `hardware_id` and `first_seen_at` out of the public response projection.
- Extended SPEC v2 gate coverage and self-test fixtures to reject legacy resource-list response assembly.

## Verification Results

- `cargo test -q --features axon-pb resource_list_entry_preserves_public_shape_without_persistence_fields --lib` — passed.
- `cargo test -q --features axon-pb resource_list_response_preserves_public_shape --lib` — passed.
- `cargo test -q --features axon-pb resource_list_entry_rejects_unknown_fields --lib` — passed.
- `cargo test -q --features axon-pb resource_list_response_rejects_unknown_fields --lib` — passed.
- `cargo test -q --features axon-pb handler_with_no_args_returns_resources_field --lib` — passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh` — passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test` — passed.
- `tools/scripts/check-architecture-convergence.sh` — passed.
- `cargo check -q --features axon-pb` — passed.
- `cargo fmt --check` — passed.
- `git diff --check` — passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph status .` — passed with index up to date.
- `/Users/macbook.silan.tech/.local/bin/codegraph affected src/daemon/ability/builtins/resources/list.rs src/daemon/resources/projection.rs src/daemon/resources/mod.rs` — no test files affected.

## Follow-up Seam

Continue moving resource ability response DTOs out of `ability/builtins/resources/*` handlers. Good next candidates are pages list/get/unpublish or files-store handlers where public response assembly still sits beside domain operation logic.
