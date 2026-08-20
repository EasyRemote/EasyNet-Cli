# System Invocation Issuer

Converge daemon-local system invocation construction behind one named
`SystemInvocationIssuer`.

V2 allows internal daemon calls only when a named issuer declares all seven
Invocation tuple fields, obtains the `_system.local` signature through the
daemon key-service path, and enters Axon's descriptor-bound LocalRuntime
admission. The current `dispatch_shim.rs` repeats local envelope assembly for
RPC, stream, and bidi helpers, each spelling out `fresh_nonce()` and
`CausalContext::None`.

This slice removes that repeated construction from the shim and makes
`local_runtime_request.rs` the single daemon/Axon bridge owner for local system
descriptor-bound requests.
