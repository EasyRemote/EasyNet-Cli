# Skill Response Envelope Boundary

## Goal

Move skill management response envelopes and receipts into the skill projection boundary. The daemon and CLI should share typed response DTOs instead of each handler or command defining ad-hoc JSON wrappers.

## Invariants

1. `InstalledSkillProjection` owns projected skill record rows.
2. `SkillRecordResponse` owns `{ ok, record }` responses for install/upgrade.
3. `SkillListResponse` owns `{ items }` responses for list.
4. `SkillRemoveReceipt` owns remove receipts, including optional `resource_ura`.
5. Ability handlers must not hand-build skill management envelopes with `json!`.
6. CLI skill commands must decode response envelopes from the projection module.
7. Public behavior remains compatible: field names stay `ok`, `record`, `items`, `name`, `agent`, `resource_ura`.

## Boundary Proof

- Store: strict persistence only.
- Projection: daemon/CLI public response DTOs.
- Ability handlers: execute operations and return serialized typed DTOs.
- CLI: invokes abilities and renders typed DTOs; it does not define duplicate response schemas.

## Verification Plan

- Projection unit tests for record/list/remove response serialization and strict decode.
- Skill remove handler test verifying typed receipt shape.
- SPEC v2 gate preventing ad-hoc skill response envelopes and duplicate CLI response schemas.
- Existing SPEC v2, architecture gate, cargo check, rustfmt, diff check, and codegraph sync/status.

## Verification Results

- `cargo test -q --features axon-pb skill_record_response_preserves_public_envelope_shape --lib`
- `cargo test -q --features axon-pb skill_list_response_preserves_items_shape --lib`
- `cargo test -q --features axon-pb skill_remove_receipt_preserves_public_shape_and_rejects_unknown_fields --lib`
- `cargo test -q --features axon-pb remove_handler_returns_typed_receipt_projection --lib`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `tools/scripts/check-architecture-convergence.sh`
- `cargo check -q --features axon-pb`
- `cargo fmt --check`
- `git diff --check`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`

## Follow-up Seam

The remaining skill-management response duplication is no longer in the install/list/upgrade/remove ability boundary. Adjacent cleanup should focus on `skill.publish` / `skill.unpublish` response receipts and skill file-operation receipts, which still assemble JSON directly inside the ability handler.
