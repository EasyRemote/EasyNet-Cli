# Invariants

1. A daemon-system remote invocation subject is target-owned: Device callees bind subject to the Device owner; Hub callees bind subject to the Hub ability URA.
2. Receipt history is session/receipt scoped. Its authority subject must match or be admitted by the session/delegation authority metadata.
3. `invocation.history.*` must never be routed through `RemoteInvocationSubject::DaemonTargetOwned`.
4. SDK history preflight remains the canonical place for language consumers to validate receipt-query authority bindings before transport.
5. Daemon admission remains the final verifier, not a compatibility repair layer for malformed product ingress.
