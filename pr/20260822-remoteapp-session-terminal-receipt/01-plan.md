# RemoteApp session terminal receipt projection

## Invariant

RemoteApp session lifecycle needs a deterministic terminal fact that product
UI and E2E harnesses can assert without guessing from the last event row.

This is not an Axon receipt replacement. Axon still owns canonical Invocation
receipts. The RemoteApp plugin owns the session lifecycle projection that says
which session terminal event closed the session.

## Change

- Add a `terminal_receipt` projection to `RemoteDesktopSession`.
- Populate it exactly once from the terminal `SESSION_CLOSED` event on explicit
  close and lease expiry.
- Expose the projection in session views.
- Gate the product-closure audit so future lifecycle work cannot remove the
  projection or its tests silently.

## Product effect

This improves the session recovery/lifecycle row by making explicit end and
timeout terminal facts inspectable. It does not implement reconnect/session
resume, consent-revoke E2E, or crash/restart recovery.
