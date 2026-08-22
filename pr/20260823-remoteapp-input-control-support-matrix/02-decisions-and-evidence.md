# Decisions and Evidence — RemoteApp Input Control Support Matrix

## Decision

- Add `metadata.input_control_support` to `device_capabilities_view()`.
- Represent support per platform and target with `available`, `permission_denied`,
  or `unsupported`.
- Keep macOS display tied to `input_injection_available()`.
- Keep macOS window/application unsupported with a target-scoped-dispatch reason.
- Keep Linux/Windows input unsupported until native input injection backends are
  implemented.

## Evidence

- Passed: `cargo test -q --features axon-pb device_capabilities_project_input_control_support_matrix --lib -- --nocapture`.
- Passed: `bash tools/scripts/check-remoteapp-lifecycle-input-boundary.sh`.
- Passed: `bash tests/scripts/test_check_remoteapp_lifecycle_input_boundary.sh`.
- Passed: `tools/scripts/check-remoteapp-main-crate-implementation-tests.sh`.
- Passed: `tools/scripts/check-remoteapp-product-closure-audit.sh`.
