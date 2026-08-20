# Remoteapp Rebind Deadline Verification

## Required checks

- Unit coverage for pending media rebind deadline expiry.
- Unit coverage for post-loss rebind deadline expiry.
- Session-store coverage that expiry rejects session rebinding and emits lifecycle evidence.
- Monitor/observer coverage that a no-observation tick still enforces expiry.
- Boundary script coverage requiring the expiry API and tests.
- Existing remoteapp target, lifecycle/input, E2E acceptance, performance, and frontend invocation checks must continue to pass.

## Commands

```sh
cargo fmt --all
cargo test -q -p easynet --features remote-desktop,headless-media target_tracking --lib
cargo test -q -p easynet --features remote-desktop,headless-media remote_desktop --lib
bash tools/scripts/check-remoteapp-lifecycle-input-boundary.sh
bash tests/scripts/test_check_remoteapp_lifecycle_input_boundary.sh
cargo test -q -p easynet --features remote-desktop,headless-media remoteapp_lifecycle_input_boundary_script_holds --test script_checks
bash tools/scripts/check-remoteapp-target-binding-boundary.sh
bash tools/scripts/check-remoteapp-e2e-acceptance-boundary.sh
bash tools/scripts/check-remoteapp-frontend-invocation-boundary.sh
bash tools/scripts/check-remoteapp-performance-boundary.sh
```

Frontend regression checks must be run from the EasyNet frontend workspace for the remote desktop protocol/store/component tests.
