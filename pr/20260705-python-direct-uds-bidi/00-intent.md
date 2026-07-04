# Python Direct UDS Bidi Intent

Implement the Python SDK direct daemon bidirectional transport required by
`docs/spec/daemon-sdk-requirements-v1.md`.

The SDK must remain a facade over the daemon-owned Axon protocol:

- `RuntimeClient.open_bidi(...)` keeps the existing SDK public surface.
- `DirectDaemonRuntimeTransport.open_bidi(...)` opens `axon.v1.Invocation.InvokeBidi`
  on the daemon invocation UDS endpoint.
- Python constructs only the daemon client frame projection needed to call the
  Axon service. It must not reimplement URA parsing, descriptor canonicalization,
  scheduling, admission, receipt validation, or bidirectional session semantics.
- `BidiSession` remains the lifecycle state machine. The direct transport only
  maps JSON frames to Axon up/down frames, enforces bounded local queues, and
  converts transport errors into `SDKError`.

This slice should move the direct daemon transport from unary + server-streaming
to unary + server-streaming + bidirectional streaming without changing the spec.
