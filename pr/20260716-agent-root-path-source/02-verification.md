# Verification

Planned checks:

- `cargo test -q list_agents_rejects_registry_rows_without_canonical_root_path --lib`
- `cargo test -q skill --lib`
- `cargo test -q ability_management --lib`
- `cargo test -q agent_registry --lib`
- `tools/scripts/check-architecture-convergence.sh`
- `tests/scripts/test_check_architecture_convergence.sh`
- `git diff --check`
