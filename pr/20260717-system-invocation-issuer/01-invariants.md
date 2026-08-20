# Invariants

- `SystemInvocationIssuer` declares caller, callee, ability descriptor ref,
  subject, nonce, causal context, and payload before dispatch.
- `_system.local` signing remains inside `LocalRuntimeRequestFactory`; the
  issuer never creates an alternate signer or cached authority.
- `dispatch_shim.rs` local RPC/stream/bidi helpers do not construct
  `DescriptorBoundEnvelopeParts` directly.
- Local system root causal context remains explicit at the call site rather
  than being hidden by a public-ingress adapter.
- External signed and bootstrap dispatch paths remain unchanged.
