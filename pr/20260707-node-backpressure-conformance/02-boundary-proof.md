# Boundary Proof

## SDK-Owned

- `StreamHandle` retained event history limit and terminal overflow projection.
- `BidiSession` retained receive/send frame limits and terminal overflow
  projection.
- Typed SDK error details for facade-observed backpressure.
- Node action-adapter report evidence for the shared conformance case.

## Runtime/Provider-Owned

- Daemon callback queue implementation.
- C ABI wire callback queue overflow delivery.
- Actual stream/bidi transport backpressure and flow-control policy.

## Product-Owned

- Browser-facing SSE/WebSocket fanout.
- Product retry UX, reconnection scheduling, and bridge-specific queue policy.

## Conclusion

Declaring Node for `stream/backpressure_bound` is valid as a seam-level facade
claim because Node exposes the shared bounded state-machine semantics over
injected transports. It remains invalid to claim daemon provider-backed Node
support until a daemon/C ABI provider exists.
