# Verification

- `cargo check -q --features axon-pb`
- `cargo fmt --check`
- `git diff --check`
- `cargo test -q skill --features axon-pb`
- `cargo test -q registration_makes_all_three_dispatchable --features axon-pb`
- `cargo test -q hub_daemon_builder --features axon-pb`
- `cargo test -q registry_assembly --features axon-pb`
- `cargo test -q device_mode_dispatcher_executes_fs_read_through_baseline_locomotion_registry --features axon-pb`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`
- `/Users/macbook.silan.tech/.local/bin/codegraph status`
