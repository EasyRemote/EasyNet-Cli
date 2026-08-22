# RemoteApp permission revoke terminal lifecycle

## Invariant

Permission revocation is a terminal RemoteApp session outcome. Once the host
reports that the selected target can no longer be captured because permission
was revoked, the same session must not remain lease-refreshable or resume
media/input under the old consent grant.

## Boundary proof

- The remote-desktop plugin owns this product lifecycle. Axon still owns only
  the surrounding Invocation and canonical receipt semantics.
- `RemoteDesktopSession` remains the single aggregate that mutates lifecycle,
  consent, target tracking, media-source loss events, and terminal receipt
  projection.
- The terminal receipt is a RemoteApp product receipt projection, not an Axon
  Invocation receipt.

## Change

- Add a stable permission-revoked terminal reason.
- When `TargetObservation::PermissionRevoked` arrives, revoke consent, emit the
  target permission event, emit media-source loss, then close the session with a
  `SESSION_CLOSED` terminal event and `terminal_receipt`.
- Preserve the existing media-source-lost return so the transport endpoint is
  still stopped by epoch.
- Update lifecycle boundary gates and tests so permission revoke cannot regress
  to a suspended non-terminal session.

## Product effect

This closes one session-recovery lifecycle seam: permission revocation now has a
deterministic terminal fact. It does not implement session resume, reconnect
handoff, crash/restart recovery, or real OS permission-revoke E2E.
