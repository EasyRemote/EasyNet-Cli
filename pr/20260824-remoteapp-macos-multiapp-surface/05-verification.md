# Verification

## Passed

- `cargo check --locked -p easynet --lib`
- `cargo fmt --all -- --check`
- `cargo test --locked -p easynet --lib daemon::plugins::remote_desktop`
  - 363 passed, 0 failed.
- Focused RemoteApp tests for target resolution, target observation, target
  tracking, input, ScreenCaptureKit capture, multi-surface composition,
  rebind failure, session behavior, global application resources, and
  deterministic epochs.
- `bash tools/scripts/check-remoteapp-target-binding-boundary.sh`
- `bash tests/scripts/test_check_remoteapp_target_binding_boundary.sh`
- `bash tools/scripts/check-remoteapp-lifecycle-input-boundary.sh`
- `bash tests/scripts/test_check_remoteapp_lifecycle_input_boundary.sh`
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh`
- `bash tests/scripts/test_check_remoteapp_product_closure_audit.sh`
- `CARGO_INCREMENTAL=0 cargo zigbuild --locked -p easynet --lib --target x86_64-pc-windows-gnu`
- `git diff --check`
- `jq empty docs/design/remoteapp-product-readiness-matrix.json`

## Environment-blocked

- `CARGO_INCREMENTAL=0 cargo zigbuild --locked -p easynet --lib --target x86_64-unknown-linux-gnu`
  reached the `wayland-sys` build script and stopped before EasyNet compilation
  because the macOS host has no Linux `pkg-config` sysroot. The local Docker
  daemon was not responsive, so an actual Linux container build could not
  replace this check in this environment. The Windows native-media cross-build
  does compile the shared non-macOS RemoteApp path.

## Product evidence still required

These implementation and conformance results do not certify product closure.
The readiness matrix remains `product_complete: false` until a real paired-host
E2E proves decoded multi-window/multi-display application video, live pointer
and keyboard input, permission-denied behavior, window-set/geometry/Z-order
rebind, disconnect/reconnect, and one terminal receipt without target widening.
