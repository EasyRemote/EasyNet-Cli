# Intent

Collapse identity-write local self authorization onto the admission transport
boundary state.

`IdentityWriteGate` currently has its own `loopback` flag and predicate. That
duplicates the local/off-box authority model already owned by
`AdmissionTransportBoundary` and makes trust-row mutation policy depend on a
second local-self interpretation.

## Expected effect

- Architecture convergence: one local-self transport boundary predicate.
- Security clarity: identity trust-row writers cannot grow an independent
  off-box bypass path.
- Public behavior unchanged for local IPC callers and strict off-box callers.
