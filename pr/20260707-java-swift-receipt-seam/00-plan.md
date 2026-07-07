# Java and Swift Receipt Seam Plan

## Goal

Move Java and Swift one step closer to the canonical SDK model by adding the
Receipt seam for fetch carriers, summary projection, opaque receipt anchors,
and causal-ref preconditions.

## Scope

- Add Java and Swift receipt fetch request DTOs with exactly-one selector
  validation.
- Add receipt summary, receipt verification, and receipt-ref DTOs.
- Add receipt clients and transports over injected transports.
- Build receipt fetch Invocation carriers from complete caller-supplied tuple
  fields and the request-supplied descriptor ref.
- Prove summary-only receipt projection cannot claim cryptographic validity.
- Prove causal refs require explicit opaque receipt URA plus hash facts.
- Keep receipt verification provider-backed; summary projection remains a local
  DTO seam only.

## Non-Goals

- No Axon/full receipt-chain cryptographic verifier.
- No daemon, C ABI, JNI, or provider transport.
- No direct ledger reads.
- No receipt URA construction.
- No product-specific receipt model.

## Capability State

Java and Swift Receipt move from `unsupported` to `seam` for fetch carriers and
summary/anchor projection. Provider-backed and cutover-ready states remain
unsupported.
