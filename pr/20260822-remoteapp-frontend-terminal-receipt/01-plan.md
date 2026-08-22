# RemoteApp frontend terminal receipt consumption

## Invariant

The frontend consumes the daemon RemoteApp `terminal_receipt` session projection
as product lifecycle state. It does not infer terminal closure by scanning the
last event row, and it does not reinterpret the projection as an Axon
Invocation receipt.

## Change

- Parse `terminal_receipt` into `RemoteDesktopView`.
- Keep the closed `end_session` response in frontend state so the terminal
  receipt remains inspectable after local transport teardown.
- Clear the session token from the retained terminal view.
- Render a compact terminal receipt marker in session details.
- Extend frontend boundary gates and tests so the field cannot silently
  disappear or regress to `session=null`.

## Product effect

This closes the UI-side half of explicit session end/timeout facts. It does not
implement reconnect/session resume, consent-revoke E2E, or crash/restart
recovery.
