# Verification

- `bash tools/scripts/check-runtime-state-read-subject-boundary.sh`
- `bash tests/scripts/test_check_runtime_state_read_subject_boundary.sh`
- `cargo fmt --check`
- `git diff --check`
- `cargo test -q pick_model --features axon-pb`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
