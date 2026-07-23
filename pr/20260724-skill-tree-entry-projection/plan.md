# Skill Tree Entry Projection

## Goal

Replace raw JSON skill tree entries and post-collection mutation with a typed `SkillTreeEntry` projection. The tree response remains public-wire compatible while entry ownership moves into the projection boundary.

## Invariants

1. `skill.tree` entries are typed projections, not raw `serde_json::Value`.
2. Each tree entry carries `path`, `type`, `size_bytes`, and `resource_ura`.
3. `resource_ura` is assigned during entry construction, not by mutating JSON after collection.
4. `.easynet/` remains excluded from tree output.
5. Public JSON field names remain unchanged.
6. No URI terminology is introduced.

## Boundary Proof

- Projection owns skill tree entry response shape.
- Publish handler owns filesystem traversal and sorting only.
- No handler-level JSON object mutation is needed for tree entries.

## Verification Plan

- Projection unit test for `SkillTreeEntry` public wire shape and strict unknown-field rejection.
- Existing tree/read handler test proving scoped tree output remains valid.
- SPEC v2 gate coverage preventing raw JSON tree entries and resource_ura mutation from returning.
- Existing SPEC v2, architecture gate, cargo check, rustfmt, diff check, and codegraph sync/status.

## Implementation Delta

- Added `SkillTreeEntry` as the projection-owned public entry shape for `skill.tree`.
- Migrated `SkillTreeResponse.files` from raw JSON values to typed `SkillTreeEntry` values.
- Removed handler-level `annotate_skill_file_resource_uras` mutation.
- Moved file/directory entry `resource_ura` construction into entry creation during traversal.
- Kept publish handler responsibility limited to path traversal, `.easynet/` exclusion, and deterministic sorting.
- Extended the SPEC v2 gate so raw tree JSON entry assembly and post-collection mutation cannot return silently.

## Verification Results

- `cargo test -q --features axon-pb skill_tree_entry_preserves_public_shape_and_rejects_unknown_fields --lib` — passed.
- `cargo test -q --features axon-pb skill_file_operation_responses_preserve_public_shapes --lib` — passed.
- `cargo test -q --features axon-pb tree_and_read_file_are_scoped_to_skill_dir --lib` — passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh` — passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test` — passed.
- `tools/scripts/check-architecture-convergence.sh` — passed.
- `cargo check -q --features axon-pb` — passed.
- `cargo fmt --check` — passed.
- `git diff --check` — passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph status .` — passed with index up to date.

## Follow-up Seam

The next resource API convergence target is any remaining handler-owned response assembly outside the skill projection boundary. Keep moving response shape ownership into typed projection modules and leave handlers responsible only for validating ingress, executing domain operations, and invoking projection constructors.
