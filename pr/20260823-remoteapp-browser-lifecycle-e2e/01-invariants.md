# RemoteApp Browser/Tauri lifecycle E2E invariants

## Evidence invariants

- `proof_mode` must be `real_browser_tauri_lifecycle`.
- `component_mock` must be false.
- `real_backend_runtime` must be true.
- `product_complete_claim` must be false.
- The evidence must include a real frontend URL and canonical device/resource
  URAs.

## Lifecycle invariants

The verifier requires this ordered sequence:

1. `app_loaded`
2. `authenticated_session`
3. `target_picker_opened`
4. `permission_status_checked`
5. `consent_granted`
6. `session_created`
7. `webrtc_attached`
8. `watch_events_streaming`
9. `media_presented`
10. `input_control_attempted_or_policy_blocked`
11. `session_ended`
12. `terminal_receipt_visible`

## Invocation invariants

- `remote_desktop.permission_status` remains host-local and must not be
  target-scoped.
- `grant_consent`, `create_session`, `attach`, `watch_events`, and
  `end_session` must bind the selected Resource URA.
- The visible terminal receipt must bind the created `session_id`.

## Product boundary

This verifier creates a place for real Browser/Tauri evidence. It does not
prove cross-device media, OS input injection, network fallback, codec soak, or
platform capture coverage by itself.
