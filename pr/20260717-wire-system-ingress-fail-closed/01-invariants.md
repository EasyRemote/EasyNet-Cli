# Invariants

1. Wire reassembly is source-neutral and fail-closed: it never invents caller,
   subject, nonce, or causal context.
2. A trusted-local transport classification does not make an incomplete
   Invocation valid.
3. Trusted-local classification mints an unforgeable crate-private authority
   seal; public SDK callers cannot select local-system ingress.
4. `_system.local` signing is inaccessible through
   `LocalRuntimeRequestFactory` outside `local_runtime_request.rs`.
5. `SystemInvocationIssuer` validates the complete envelope caller and delegates
   to the same descriptor-bound Axon request constructor used by external
   admission.
6. External, bootstrap, and local-system requests preserve their existing
   descriptor ref, payload, trace id, and admitted metadata.
7. Missing caller, subject, or nonce fails before replay admission or handler
   execution.
8. No compatibility fallback remains after callers migrate.
