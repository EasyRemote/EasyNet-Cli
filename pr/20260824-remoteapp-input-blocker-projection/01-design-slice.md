# Design Slice

## Runtime ownership

- `TargetTrackerSnapshot` remains the source of target readiness truth.
- The session view projects its exact `input_blocked_reason` rather than a
  second generic classification.
- The platform input backend projects its typed unavailable reason, including
  macOS `accessibility_permission_denied`.
- `TARGET_BLURRED` keeps media alive and input disabled, but its frontend action
  is `focus_target_locally`, not `retry_session`.

## Evidence contract

The Browser/Tauri lifecycle verifier accepts the typed fail-closed reasons used
by the runtime. A policy-blocked artifact must also carry the safe target
tracking projection exposed by the frontend: binding state, visibility, focus,
input-enabled state, focus epoch, and geometry revision. Session tokens are not
exposed.

## Non-claims

This slice does not claim applied input. Applied input still requires a real
`INPUT_FRAME_APPLIED` event, independent OS effect, permission evidence, focus
epoch, geometry revision, and bounded latency.
