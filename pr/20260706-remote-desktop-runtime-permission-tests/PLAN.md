# Remote Desktop Runtime Permission Test Plan

## Objective

Align remote desktop media backend tests with the runtime permission state
machine already implemented in the daemon media catalog.

## Boundary

- Do not change `docs/spec/daemon-sdk-requirements-v1.md`.
- Do not change media backend production selection logic in this slice.
- Keep the native macOS plugin descriptor compiled in on macOS, while tests
  accept that runtime Screen Recording permission can make it unavailable.

## Invariants

1. Native ScreenCaptureKit + VideoToolbox is production-ready only when the
   runtime descriptor is available.
2. Permission denial must close `production_gate_view()` and expose
   `screen_capture_permission_denied`.
3. WebRTC transport can still select the baseline xcap/OpenH264 path when the
   native production backend is unavailable.
4. Non-macOS behavior remains `not_installed`.

## Verification

- `cargo test --lib catalog_declares_native_plugin_state_per_platform`
- `cargo test --lib native_plugin_runtime_permission_controls_production_gate_on_macos`
- `cargo test --lib direct_webrtc_can_start_before_flagship_native_backend_is_available`
- `cargo fmt --check`
- `git diff --check`
