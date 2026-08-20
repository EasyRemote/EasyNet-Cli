# Invariants

- Unary, server-stream, and bidi exact daemon routes have typed route owners in
  `daemon_invocation_service.rs`.
- Runtime-admin bidi conformance consumes the production route inventory rather
  than a second hand-maintained string list.
- `BidiDispatcher` classifies exact bidi routes through `DaemonBidiRoute`, not
  through direct `match ability_name` string dispatch.
- V2 convergence runs the architecture route inventory gate, so RF-7/RF-8
  route ownership regressions fail the V2 gate.
- `session.open` remains explicitly identified as further cutover work; this
  slice does not introduce a compatibility fallback or claim terminal
  LocalRuntime finalization for that carrier.
