# Decisions and Evidence — RemoteApp Platform Capability Claims

## Decision

- Keep the native macOS backend descriptor in the backend catalogue so operators
  can see why flagship capture is blocked.
- Change device capability projection so `metadata.production_target_subjects`
  is derived from `production_ready`, not raw descriptor subjects.
- Derive that state from the same runtime native backend descriptor used by the
  production gate, so macOS Screen Recording permission denial and non-macOS
  `not_installed` state are reflected consistently.
- Add separate `metadata.diagnostic_target_subjects` for the display-only xcap
  fallback.
- Add tests proving production subjects are empty when the production gate is
  closed and display-only diagnostic subjects remain visible.

## Product effect

- The frontend/product layer can no longer treat unavailable macOS-native
  window/application capture as currently product-ready on Linux, Windows, or
  permission-denied macOS.
- This is a correctness fix for capability claims; real Windows/Linux
  app/window capture remains incomplete until a native backend or an explicit
  live unsupported artifact is provided.

## Evidence

- Passed: `cargo test -q --features axon-pb device_capabilities_project_native_target_subject_matrix --lib -- --nocapture`.
- Passed: `tools/scripts/check-remoteapp-main-crate-implementation-tests.sh`.
- Passed: `tools/scripts/check-remoteapp-product-closure-audit.sh`.
