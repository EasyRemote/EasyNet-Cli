# Verification

- `cargo fmt --check -- src/daemon/ability/builtins/resources/media/camera_snapshot.rs src/daemon/ability/builtins/resources/media/avfoundation_camera.rs`: passed.
- `cargo test -p easynet --features native-media --lib snapshot_subscription_reuses_only_a_live_camera_producer`: passed.
- `cargo check -p easynet --features native-media`: passed.
- `cargo test -p easynet --features native-media --lib physical_macos_snapshot_reuses_the_live_preview_frame -- --ignored --nocapture`: passed against the physical macOS camera in 1.45 seconds.
- `packaging/release/dev-install-local.sh --debug`: both builds passed; installation stopped at the macOS sudo password prompt, so the running root-owned daemon was not replaced.
