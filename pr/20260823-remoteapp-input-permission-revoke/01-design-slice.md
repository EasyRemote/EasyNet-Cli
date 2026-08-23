# RemoteApp input permission revoke slice

Date: 2026-08-23

## Product requirement

Interactive RemoteApp sessions must remain permission-correct after startup.
Checking OS input permission when the data channel opens is insufficient:
Accessibility/input injection permission can be revoked while media continues.

## Boundary

- The RemoteApp plugin owns OS-local pointer/keyboard execution and input
  lifecycle projection.
- The session aggregate owns lifecycle transitions and event ordering.
- Media transport remains active; only input is deactivated until permission is
  re-proven.

## Implemented slice

- Detect host-side input permission denial from the input execution path.
- Project a session-level `INPUT_PERMISSION_BLOCKED` event for the current
  transport epoch.
- Downgrade lifecycle from `InputActive` back to `MediaActive` without closing
  media.
- Keep per-frame `INPUT_FRAME_REJECTED` diagnostics for probe correlation.

## Non-claims

This does not prove a real user revoked Accessibility permission during a live
macOS session. It makes that execution result visible and bounded when the OS
input backend reports the denial.
