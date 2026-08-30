# Verification

## Passed

- `cargo check --locked -p easynet`
- `cargo test --locked -p easynet application_compositor -- --nocapture`
- `bash tools/scripts/check-remoteapp-target-binding-boundary.sh`
- `bash tools/scripts/check-media-screen-target-provider-boundary.sh`
- `bash tools/scripts/check-remoteapp-lifecycle-input-boundary.sh`
- `bash tools/scripts/check-remoteapp-main-crate-implementation-tests.sh`
- `bash tests/scripts/test_check_remoteapp_target_binding_boundary.sh`
- `bash tests/scripts/test_check_remoteapp_lifecycle_input_boundary.sh`
- `git diff --check`

Focused target-binding, target-observer, media-selector, and capability tests are
run again after final formatting before commit.

## Environment-blocked

- `cargo check --locked -p easynet --target x86_64-unknown-linux-gnu` reaches
  dependency compilation but cannot compile `ring` because this macOS host does
  not have `x86_64-linux-gnu-gcc` installed.
- Docker validation is not recorded because the local Docker engine did not
  return a usable `docker version` response.

## Still required for product certification

Run `remoteapp-cross-platform-capture-e2e.sh` on real Windows and Linux hosts and
retain decoded-frame, sentinel leakage, target rebind, and terminal receipt
artifacts. Source tests and verifier self-tests do not replace this evidence.
