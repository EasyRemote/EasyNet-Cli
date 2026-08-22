# Decisions and Evidence — RemoteApp Platform Support Matrix

## Decision

- Add `metadata.platform_support` to `device_capabilities_view()`.
- Represent each platform row as product-visible target statuses:
  `production_ready`, `blocked`, `diagnostic_only`, or `unsupported`.
- Keep Linux display distinct from Linux app/window: display can be diagnostic
  only, while app/window are unsupported.
- Keep Windows explicitly unsupported for every target kind.

## Evidence

- Passed: `cargo test -q --features axon-pb device_capabilities_project_cross_platform_support_matrix --lib -- --nocapture`.
- Passed: `bash tools/scripts/check-remoteapp-lifecycle-input-boundary.sh`.
- Passed: `bash tests/scripts/test_check_remoteapp_lifecycle_input_boundary.sh`.
- Passed: `tools/scripts/check-remoteapp-main-crate-implementation-tests.sh`.
- Passed: `tools/scripts/check-remoteapp-product-closure-audit.sh`.
