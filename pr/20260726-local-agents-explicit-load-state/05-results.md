# Results

Implemented explicit hosted-agent identity projection load state.

## Refactoring

- Added `LocalAgentsLoadState::{Loaded, Missing}`.
- Added `load_with_state()` as the storage reader.
- Added `load_for_fresh_host_projection()` as the explicit first-boot
  projection helper.
- Preserved `load()` as the stable public read projection.
- Migrated agent lifecycle and aggregate snapshot production paths to
  `load_for_fresh_host_projection()`.

## Tests

- Added `missing_file_projects_explicit_load_state`.
- Added `first_boot_projection_returns_empty_identity_registry`.
- Added `existing_file_projects_loaded_state`.
- Added `existing_malformed_file_fails_closed`.

## Verification

- `cargo test daemon::persistence::local_agents::tests --lib`
- `cargo test daemon::persistence::agent_aggregate::tests --lib`
- `cargo test daemon::ability::builtins::agents::lifecycle::tests::startup_bootstrap_projection_persists_through_lifecycle_owner --lib`
- `cargo fmt --check`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-architecture-convergence.sh`
- `git diff --check`

## Gate

- SPEC v2 now retires direct `LocalAgentsFile::default()` returns from the
  storage reader and requires explicit load-state modeling.
- Architecture convergence now requires the Agent aggregate repository to use
  the explicit first-boot projection boundary.
