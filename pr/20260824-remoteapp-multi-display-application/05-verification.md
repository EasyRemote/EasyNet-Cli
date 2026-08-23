# Verification

Passed:

- `cargo test --locked -p easynet application_compositor --lib` — 4 passed.
- `cargo test --locked -p easynet device_capabilities_project_cross_platform_support_matrix --lib` — 1 passed.
- `bash tools/scripts/check-remoteapp-lifecycle-input-boundary.sh`.
- `bash tests/scripts/test_check_remoteapp_lifecycle_input_boundary.sh`.
- `bash tools/scripts/check-remoteapp-target-binding-boundary.sh`.
- `bash tests/scripts/test_check_remoteapp_target_binding_boundary.sh`.
- `bash tools/scripts/check-media-screen-target-provider-boundary.sh`.
- `bash tools/scripts/check-remoteapp-main-crate-implementation-tests.sh`.
- `cargo check --locked -p easynet --lib`.
- `cargo zigbuild --target x86_64-unknown-linux-gnu --locked -p easynet --lib --no-default-features --features axon-pb,remote-desktop,headless-media`.
- `cargo zigbuild --target x86_64-pc-windows-gnu --locked -p easynet --lib --no-default-features --features axon-pb,remote-desktop,headless-media`.
- `git diff --check`.

The cross-builds prove compile-time platform coverage only. They do not replace
live Windows/Linux decoded-frame/leakage/rebind artifacts or a macOS
multi-display `MultiAppSurface` implementation.
