# Architecture

The boundary remains:

```text
Axon protocol helpers
  -> EasyNet-Cli daemon / Rust / C ABI projection
  -> Python SDK facade DTOs and clients
  -> EasyRemote product facade
```

The implementation extends the existing `ConsumerBoundaryAuditor` contract
rather than adding another scanner. This keeps all EasyRemote SDK-consumer
rules in one object and lets the shell gate, Python tests, and conformance
manifest share the same evidence source.

No new protocol data path is added. The work only makes existing cutover
expectations explicit and executable.
