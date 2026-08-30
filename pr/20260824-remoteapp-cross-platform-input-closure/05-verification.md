# Verification

## Passed

- `cargo check --locked -p easynet`
- `cargo zigbuild --target x86_64-unknown-linux-gnu --locked -p easynet --lib --no-default-features --features axon-pb,remote-desktop,headless-media`
- `cargo test --locked -p easynet remote_desktop::input --lib` — 31 passed
- `cargo test --locked -p easynet remote_desktop::view_device --lib` — 6 passed
- `tools/scripts/check-remoteapp-platform-input-backends.sh`
- `tests/scripts/test_check_remoteapp_platform_input_backends.sh`
- `bash tools/scripts/check-remoteapp-lifecycle-input-boundary.sh`
- `bash tests/scripts/test_check_remoteapp-lifecycle-input-boundary.sh`
- `cargo fmt --all -- --check`
- `git diff --check`

## Cross-platform build boundary

The Windows `cargo zigbuild --target x86_64-pc-windows-gnu --locked -p easynet
--lib --no-default-features --features
axon-pb,remote-desktop,headless-media` compiled the new User32 backend without a
diagnostic, then stopped at two existing main-crate Windows `cfg` gaps:

1. `host_stream.rs` imports `tokio::net::UnixStream` on Windows.
2. `AgentRootIdentity::matches_metadata` is not compiled for Windows while a
   Windows-visible lifecycle call references it.

This is not recorded as a passing Windows build. Those main-crate blockers and
real Windows/Linux OS-effect artifacts remain required before product-complete
claims. Linux Wayland also remains unavailable until the portal RemoteDesktop
session is bound to the selected Resource and lifecycle.
