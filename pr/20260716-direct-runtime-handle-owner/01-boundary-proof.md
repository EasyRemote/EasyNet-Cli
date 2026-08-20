# Boundary Proof

- Owner: `RuntimeTransport` injected as `HandleTransport` owns `Prepare`,
  `SubmitSigned`, `AwaitHandle`, `CancelHandle`, `HandleEvents`, and
  `FreeHandle`.
- Direct transport state machine: `open -> invoke | stream | bidi -> close`.
  It has no local prepared/submitted/terminal-handle state.
- Missing owner transition: any handle operation without `HandleTransport`
  fails with `NOT_IMPLEMENTED`; it does not execute a local substitute.
- Invariant: Go and Python direct transports report `prepare=false` and
  `submit_signed=false` at handshake exactly when these calls fail closed.
