# Canonical Runtime Convergence V2 - Decisions

## 2026-07-17

- The clean target removes Mission/EAL from Axon core but keeps generic child
  invocation, causal-chain, cancellation, deadline, and receipt semantics in
  Axon.
- Daemon-internal calls remain valid only as explicit system invocations signed
  by the daemon key service. This is not an SDK signer fallback.
- A language facade is not cutover-ready merely because it has matching method
  names. It must pass the shared lifecycle transition and recovery vectors.
- Existing public compatibility can be release-scoped at an edge adapter, but
  it cannot preserve a second canonical admission, signer, or receipt path.
