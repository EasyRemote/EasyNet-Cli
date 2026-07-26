# Decisions Log

- Decision: keep `CanonicalRuntimeReceiptResolver::new()` infallible but make its realm trust source explicit.
  - Reason: existing FFI and dispatch callers expect infallible construction; the trust failure belongs in signer resolution where the signer URA is known.
