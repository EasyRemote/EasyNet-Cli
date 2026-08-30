# Verification

- `bash tools/scripts/host-remoteapp-decoded-frame-e2e.sh --self-test`
- `bash tools/scripts/host-remoteapp-permission-subject-e2e.sh --self-test`
- `bash tools/scripts/host-remoteapp-decoded-frame-e2e.sh --run --target-kind window --sentinel-fixture`
  - result before this patch: passed
- `bash tools/scripts/host-remoteapp-decoded-frame-e2e.sh --run --target-kind application --sentinel-fixture`
  - result before this patch: failed after decoding 50 frames with
    `selected_content_present=false`

Post-patch checks to run:

- `cargo fmt --all`
- `cargo test -q -p easynet --features remote-desktop,native-media application_capture_uses_screencapturekit_application_filter_contract --lib`
- `cargo test -q --manifest-path plugins/remote-desktop/Cargo.toml --lib`
- `bash tools/scripts/check-remoteapp-performance-boundary.sh`
- `bash tools/scripts/check-remoteapp-picker-subject-boundary.sh`
- `bash tools/scripts/check-remoteapp-e2e-acceptance-boundary.sh`
- `bash tools/scripts/check-remoteapp-target-binding-boundary.sh`
- `bash tools/scripts/host-remoteapp-decoded-frame-e2e.sh --run --target-kind application --sentinel-fixture`
  - result: passed; report at `target/e2e/host-remoteapp-decoded-frame/20260821-124547-application-28636/report.md`
- `bash tools/scripts/host-remoteapp-decoded-frame-e2e.sh --run --target-kind window --sentinel-fixture`
  - result: passed; report at `target/e2e/host-remoteapp-decoded-frame/20260821-124605-window-29771/report.md`
