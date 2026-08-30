# Plan — RemoteApp Frontend Offline Resume Gate

## Product invariant

Device presence loss is not a RemoteApp terminal event. The frontend may close
browser-local transport when a device goes offline, but it must preserve a
non-terminal daemon RemoteApp session until the daemon reports a terminal
session state or the user explicitly ends the session.

## Boundary

- EasyNet-Cli owns the product contract and cross-repo gate.
- EasyNet frontend owns browser-local media/WebRTC state and user-visible
  RemoteApp lifecycle projection.
- Runtime/Axon invocation semantics are unchanged. Resume still uses normal
  RemoteApp abilities and canonical invocation metadata.

## Required frontend behavior

1. `suspendEntryForOffline` preserves non-terminal RemoteApp session state.
2. `DeviceMediaAccess` must not call `rdEnd` merely because `online === false`.
3. `resumeEntryFromOffline` resumes only preserved non-terminal RemoteApp
   sessions.
4. Resume validates the daemon session with `remote_desktop.show_session`.
5. Resume rebinds WebRTC with transport-failure cleanup configured to preserve
   the daemon session for another reconnect attempt.
6. Resume restarts session event watching and lease refresh after WebRTC
   negotiation.

## Gate updates

- `tools/scripts/check-remoteapp-frontend-invocation-boundary.sh` rejects:
  session clearing during offline suspend, missing resume validation,
  end-session cleanup during resume transport failure, and UI offline effects
  that call `rdEnd`.
- `tests/scripts/test_check_remoteapp_frontend_invocation_boundary.sh` adds
  positive fixture coverage and mutation tests for the same regressions.

## Product effect

Short frontend/device presence drops no longer destroy valid RemoteApp sessions.
This does not prove long network outage recovery, NAT/relay handoff,
process crash/restart recovery, or real cross-device media/input E2E.
