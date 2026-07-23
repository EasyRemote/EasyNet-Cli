# Verification

## Planned checks

- `cargo fmt --check`
- `cargo test --features axon-pb runtime_session_projection --lib`
- `cargo test --features axon-pb runtime_stop_plan --lib`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-architecture-convergence.sh`
- `git diff --check`

## Results

- `cargo fmt --check` passed.
- `cargo test --features axon-pb projection_json_preserves_runtime_kind_wire_name --lib` passed.
- `cargo test --features axon-pb load_current_rejects_malformed_existing_projection --lib` passed.
- `cargo test --features axon-pb stop_plan_treats_projection_missing_live_daemon_as_daemon_only --lib` passed.
- `cargo test --features axon-pb stop_plan_maps_runtime_projection_to_daemon_only --lib` passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` passed.
- `bash tools/scripts/check-architecture-convergence.sh` passed.
- `git diff --check` passed.
