# Decisions and Evidence — RemoteApp Capability Gate Hardening

## Decision

- Extend `check-remoteapp-lifecycle-input-boundary.sh` so it pins
  runtime-native descriptor usage and production-vs-diagnostic target-subject
  separation.
- Update the script mutation fixture and failure cases so stale raw descriptor
  projection no longer satisfies the gate.

## Evidence

- Passed: `bash tests/scripts/test_check_remoteapp_lifecycle_input_boundary.sh`.
- Passed: `bash tools/scripts/check-remoteapp-lifecycle-input-boundary.sh`.
- Passed: `tools/scripts/check-remoteapp-main-crate-implementation-tests.sh`.
- Passed: `tools/scripts/check-remoteapp-product-closure-audit.sh`.
