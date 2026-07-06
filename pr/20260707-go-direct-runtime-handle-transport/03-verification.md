# Verification

## 2026-07-07

- `go test . -run 'TestDirectDaemonRuntime'` in `sdk/go`: passed.
- `go test .` in `sdk/go`: passed.

Covered facts:

1. Direct daemon runtime unary/stream/bidi behavior still passes existing tests.
2. Direct daemon runtime delegates prepare, submit, await, cancel, events, and
   free-handle calls to the configured `RuntimeTransport`.
3. Direct runtime handshake advertises prepare/submit only when a handle
   transport is present.
4. Explicit handle-transport ownership closes the delegated transport exactly
   once.
5. `CompatibilityClient.RetrieveFile` has a MEMC owner mapping.
