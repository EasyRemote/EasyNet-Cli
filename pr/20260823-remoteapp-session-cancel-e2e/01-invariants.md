# RemoteApp session cancel E2E invariants

## Invocation and ownership boundaries

- `create_session` and `end_session` must act on the selected Resource URA via
  Invocation `subject`.
- `create_session` args must not duplicate subject identity as `subject`,
  `subject_ura`, or `resource_ura`.
- The callee remains the RemoteDesktop ability owner; the target
  display/window/application remains the resource subject.
- The E2E must use public CLI/daemon abilities rather than internal handler
  calls.

## Lifecycle invariants

- First `remote_desktop.end_session` with reason `user_cancelled` must close the
  session.
- Public `show_session` after cancel must project the same terminal receipt.
- A repeated `end_session` after cancel must return `already_ended=true`.
- Repeated end must preserve the original `terminal_receipt`; it must not create
  a second terminal fact or overwrite the reason.

## Evidence boundaries

- This harness proves product-level user cancel/close lifecycle for a local host
  target.
- It does not prove Axon transport cancellation, cross-device reconnect,
  crash/restart recovery, consent revoke handling, or successful interactive
  input injection.
