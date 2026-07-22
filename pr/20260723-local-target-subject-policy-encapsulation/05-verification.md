# Verification

Completed:

- `cargo test -q local_system_context_for_agent_target_uses_agent_owner_subject --lib`
- `cargo test -q local_system_context_for_hub_target_uses_ability_subject --lib`
- `cargo test -q pages_ability_targets_pages_agent_callee --lib`
- `cargo test -q principal_get_target_uses_explicit_top_level_read_schema --lib`
- `cargo fmt --check`
- `bash tools/scripts/check-daemon-invocation-migration.sh`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `codegraph sync`
- `codegraph query default_subject_ura --limit 20`

Result:

- Rust focused tests passed.
- `check-daemon-invocation-migration.sh` passed.
- `check-architecture-convergence.sh` passed.
- `check-canonical-runtime-convergence-v2.sh` passed.
- codegraph reports no `default_subject_ura` symbol.
