# Invariants

## Semantic Invariants

- Invocation nonce is caller-provided freshness material and must be exactly 16 bytes after base64 decoding.
- Validation belongs at the Runtime Core DTO boundary because every transport variant consumes the same `InvocationDraft`.
- Prepared signing material may perform deeper canonical checks, but it must not be the first boundary that catches malformed tuple freshness.

## Boundary Invariants

- Go and Python expose the same public field: `nonce_base64`.
- The SDK remains product-neutral; validation references only generic Invocation semantics.
- The SDK does not introduce alternate URI naming; all routable identity fields remain URA fields.

## Boundedness Invariants

- Decode work is bounded by one base64 decode of a single nonce string.
- Rejection is deterministic and has no transport, filesystem, or daemon dependency.
