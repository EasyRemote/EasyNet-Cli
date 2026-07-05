# Invariants

- Carrier construction remains in Rust daemon SDK contract and delegates
  descriptor construction to the shared system-invocation builder.
- Go/Python C ABI transports run build -> Runtime Core invoke -> Rust-owned
  projection for every hub/pairing/credential method.
- Hub URAs and device URAs are validated before carrier construction.
- Pairing tokens, credentials, and verification results are projected from
  daemon output plus explicit request context; SDK facades do not invent trust.
- The previous semantic-boundary `NOT_IMPLEMENTED` branch is removed only for
  operations whose Rust/C ABI contract exists in this slice.
