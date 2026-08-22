# RemoteApp cross-platform capture evidence invariants

## Evidence invariants

- `proof_mode` must be `real_cross_platform_capture_matrix`.
- `component_mock` must be false.
- `real_backend_runtime` must be true.
- `product_complete_claim` must be false.
- The artifact must include platform entries for `macos`, `windows`, and
  `linux`.

## Capture invariants

For a passing platform/target scenario:

- `target_kind` must be `display`, `window`, or `application`.
- `selected_resource_ura` must be canonical.
- public `remote_desktop.create_session`, `remote_desktop.attach`,
  `remote_desktop.watch_events`, and `remote_desktop.end_session` evidence must
  bind the selected Resource URA.
- `capture_backend` must be explicit.
- `capture_scope` must match the target kind.
- `frames_rendered` and `duration_ms` must be positive.
- `terminal_receipt` must be visible and bind the same session id.
- window/application scenarios must prove `first_display_capture_started=false`
  and must not use display fallback.

## Platform invariants

- macOS must pass display/window/application capture.
- Windows and Linux may pass capture or report explicit product unsupported
  state for each target kind.
- Unsupported state must include `status=unsupported`, `unsupported_state`
  equal to `explicit_product_unsupported`, `show_unsupported=true`, no rendered
  frames, and no capture session id.

## Product boundary

This verifier defines the cross-platform capture artifact required before
product completion. It does not create real platform evidence by itself.
