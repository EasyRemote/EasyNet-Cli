# Skill Publish Response Boundary

## Goal

Move `skill.publish`, `skill.unpublish`, `skill.tree`, `skill.read_file`, and `skill.write_file` response receipts into the skill projection boundary. This removes ad-hoc response JSON from the handler layer while preserving public wire fields.

## Invariants

1. Skill publish/file-operation handlers execute operations; they do not own response schema construction.
2. Public field names remain stable: `ok`, `owner_agent_id`, `skill_name`, `skill_dir`, `removed_dir`, `content_hash`, `resource_ura`, `files`, `path`, `content`, `encoding`, `size_bytes`.
3. Response DTOs reject unknown fields.
4. Tree entries may remain JSON values in this slice; the response envelope is still typed.
5. No URI terminology is introduced.

## Boundary Proof

- Store owns install provenance.
- Projection owns daemon/CLI public skill response DTOs.
- Publish/file handlers own filesystem mutation and validation only.

## Verification Plan

- Projection unit tests for publish/unpublish/tree/read/write response shapes.
- Existing skill publish/file-operation handler tests must continue passing.
- SPEC v2 gate coverage preventing ad-hoc publish/file response assembly from returning.
- Existing SPEC v2, architecture gate, cargo check, rustfmt, diff check, and codegraph sync/status.

## Verification Results

- `cargo test -q --features axon-pb skill_publish_and_unpublish_receipts_preserve_public_shapes --lib`
- `cargo test -q --features axon-pb skill_file_operation_responses_preserve_public_shapes --lib`
- `cargo test -q --features axon-pb tree_and_read_file_are_scoped_to_skill_dir --lib`
- `cargo test -q --features axon-pb write_file_updates_content_and_rejects_traversal --lib`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `tools/scripts/check-architecture-convergence.sh`
- `cargo check -q --features axon-pb`
- `cargo fmt --check`
- `git diff --check`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`

## Follow-up Seam

The skill package response envelopes are now centralized. Remaining adjacent cleanup is inside tree entry modeling: `collect_skill_tree_entries` still emits raw JSON entry values and `annotate_skill_file_resource_uras` mutates those entry objects. That should become a typed `SkillTreeEntry` projection in a later slice.
