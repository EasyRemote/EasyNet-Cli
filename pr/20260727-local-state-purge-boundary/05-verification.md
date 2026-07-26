# Verification

- `cargo fmt --check` passed.
- `cargo test -q reset --features axon-pb` passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh` passed.
- `tools/scripts/check-architecture-convergence.sh` passed.
- `codegraph sync .` completed after the code changes.

## Coverage

- `reset_purge_local_state_removes_keyring_descriptor_and_registry_root`
  proves purge mode removes the local state root rather than only
  `credentials.json`.
- `local_state_purge_root_rejects_relative_and_non_easynet_paths` proves the
  destructive purge boundary rejects unsafe roots.
- SPEC v2 gate `check_reset_local_state_purge_boundary_contract` rejects a
  future return to credentials-only purge behavior or legacy file enumeration.
