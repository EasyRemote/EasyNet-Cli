# Invariants

- `local_runtime_invoker.rs` resolves callee, subject, causal context, call
  mode, descriptor ref, payload bytes, and request metadata.
- `local_runtime_invoker.rs` does not construct `DescriptorBoundEnvelopeParts`,
  call `fresh_nonce()`, or mint `_system.local` caller identity.
- `SystemInvocationIssuer` remains the only daemon-local constructor for
  `_system.local` descriptor-bound request envelopes.
- Existing RPC, stream, and bidi dispatch behavior remains public-interface
  compatible.
- Focused tests assert the same callee, subject, descriptor ref, and finalized
  RPC output through the issuer-created request.
