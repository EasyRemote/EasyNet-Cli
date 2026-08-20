# Verification

Passed:

- `bash tools/scripts/check-runtime-state-read-subject-boundary.sh`
- `bash tests/scripts/test_check_runtime_state_read_subject_boundary.sh`
- `cargo fmt --check`
- `git diff --check`
- `cargo test -q user_signing --features axon-pb`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`
- `/Users/macbook.silan.tech/.local/bin/codegraph status`

Source scan:

- `src/cli/commands/user_signing_identity.rs` has no
  `invoke_local_ability` production call.
- `identity.list_user_pubkeys` is now reached through
  `LocalRuntimeStateReadIssuer`; `identity.register_pubkey` remains on
  `LocalDaemonSystemAbilityIssuer`.
