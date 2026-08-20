# Skill Record Projection Boundary

## Goal

Split the skill install persistence schema from daemon/CLI response projection. `InstallRecord` remains the strict on-disk schema; public skill ability responses use an explicit projection type that owns optional `resource_ura`.

## Invariants

1. `InstallRecord` must not own `resource_ura` or other response-only fields.
2. Ability handlers must not serialize an `InstallRecord` and mutate the resulting JSON object to add projection fields.
3. CLI skill commands must decode daemon responses through the projection type, not through persistence records.
4. Public JSON compatibility remains intact: `content_hash` stays the wire field, and `resource_ura` remains optional on response rows.
5. Projection decoding is strict: unknown response fields fail closed unless deliberately modeled.

## Boundary Proof

- `daemon::resources::skills::store` owns install provenance persistence.
- `daemon::resources::skills::projection` owns public daemon/CLI skill record views.
- Ability handlers and CLI commands consume projections; only store helpers read/write `InstallRecord`.

## Verification Plan

- Rust unit tests for projection serialization and strict unknown-field rejection.
- Rust tests pinning `skill.install` response projection and `skill.list` projected `resource_ura`.
- SPEC v2 gate coverage preventing JSON mutation of persistence records and preventing CLI response decoding into `InstallRecord`.
- Existing SPEC v2, architecture gate, cargo check, rustfmt, diff check, and codegraph sync/status.

## Verification Results

- `cargo test -q --features axon-pb installed_skill_projection --lib`
- `cargo test -q --features axon-pb project_install_record_returns_response_projection_with_resource_ura --lib`
- `cargo test -q --features axon-pb list_handler_projects_resource_ura_without_extending_install_record_schema --lib`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `tools/scripts/check-architecture-convergence.sh`
- `cargo check -q --features axon-pb`
- `cargo fmt --check`
- `git diff --check`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`

## Follow-up Seam

`skill.remove` still builds its small receipt with direct JSON object assembly. It no longer mutates an install record, so it is outside this boundary, but a later cleanup should give skill management receipts explicit response structs across install/list/upgrade/remove.
