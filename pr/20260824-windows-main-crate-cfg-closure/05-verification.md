# Verification

- `tools/scripts/check-windows-main-crate-platform-boundaries.sh` — passed
- `tests/scripts/test_check_windows_main_crate_platform_boundaries.sh` — passed
- `cargo check --locked -p easynet` — passed
- `cargo zigbuild --target x86_64-pc-windows-gnu --locked -p easynet --lib
  --no-default-features --features axon-pb,remote-desktop,headless-media` — passed
- `cargo fmt --all -- --check` — passed
- `git diff --check` — passed

The Windows command proves source/build closure for this feature set. It does
not prove a real Windows host can enumerate, capture, focus, or inject the
selected RemoteApp Resource; those remain live-host E2E requirements.
