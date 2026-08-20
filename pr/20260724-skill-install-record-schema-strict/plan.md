# Skill Install Record Schema Strictness

## Goal

Remove the implicit compatibility layer around `.easynet/install.json` skill records. Public JSON keeps the current `content_hash` field, but active persistence must reject unknown/legacy fields and callers must not silently swallow malformed install records as if they were valid canonical state.

## Invariants

1. `InstallRecord` is the canonical skill install persistence schema.
2. `SkillSource` is nested canonical provenance and must reject unknown fields.
3. The public/wire field remains `content_hash`; no `skill_tree_hash` JSON field is introduced.
4. Unknown top-level or nested install-record fields fail closed.
5. Destructive unpublish may still remove an existing directory, but malformed provenance must be explicit in the audit hash rather than silently treated as canonical data.

## Boundary Proof

- Skill store owns install provenance persistence.
- Skill abilities may project or mutate records only through the strict store model.
- Backend/frontend compatibility is limited to the intentional `content_hash` public field name, not open-ended unknown-field tolerance.

## Verification Plan

- Targeted Rust tests for install-record strict unknown-field rejection.
- Targeted Rust test for unpublish malformed provenance audit marker.
- Canonical runtime convergence v2 gate coverage for strict skill install schema.
- Architecture convergence gate, rustfmt, diff check, codegraph sync/status.

## Verification Results

- `cargo test -q --features axon-pb install_record --lib`
- `cargo test -q --features axon-pb unpublish_marks_malformed_install_record_without_accepting_legacy_fields --lib`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `tools/scripts/check-architecture-convergence.sh`
- `cargo check -q --features axon-pb`
- `cargo fmt --check`
- `git diff --check`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`

## Follow-up Seam

`codegraph impact InstallRecord` shows the next architectural seam: daemon skill abilities and the CLI still use `InstallRecord` as both the persistence schema and the public response record. That coupling should be split into a strict persistence model plus a small response/view DTO before `record.resource_ura` or other projected fields grow further.
