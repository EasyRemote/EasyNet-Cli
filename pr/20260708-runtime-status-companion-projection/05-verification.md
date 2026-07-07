# Verification

- `cargo fmt --check`
  - Passed.
- `cargo test -q daemon::boot::lifecycle::status`
  - Passed: 6 tests.
- `cargo test -q daemon::boot::lifecycle::service`
  - Passed: 2 tests.
- `cargo test -q daemon::plugins::companion`
  - Passed: 32 tests.
- `git diff --check`
  - Passed.
- `rg -n "\b[U]R[I]\b|\bu[r]i\b" src/daemon/boot/lifecycle/status.rs src/daemon/boot/lifecycle/service.rs pr/20260708-runtime-status-companion-projection -g '!target'`
  - Passed: no matches.
