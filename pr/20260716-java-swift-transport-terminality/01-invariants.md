# Invariants

- Runtime terminal state remains represented by `terminal`.
- Local transport closure is represented by `transportTerminal`.
- `cancel`, `closeSend` and backpressure do not populate runtime terminal
  accessors.
- Retained frame/event buffers remain bounded and keep the transport-terminal
  sentinel for diagnostics.
- Java and Swift expose the same semantic split and their tests cover both
  stream and bidi paths.
- Public API inventory and parity matrix must be regenerated after the public
  surface changes.
