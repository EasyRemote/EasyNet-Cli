# Boundary Proof

## Ownership

Receipt semantics remain owned by daemon/Axon provider paths. Java and Swift are
P1 injected-transport seams that validate request/projection DTO shape, delegate
provider-owned operations, and parse returned projections.

Authority signing, verification, and trust admission remain daemon/Axon-owned.
Java and Swift expose typed authority metadata envelopes and minting clients
over injected transports so Invocation builders can preserve metadata without
duplicating provider policy.

## Invariants

- Receipt summary projection never claims cryptographic verification.
- `ReceiptRef` requires both `receipt_ura` and `receipt_hash_hex`.
- Causal references are delegated to the receipt provider and are never
  constructed by Java or Swift facade code.
- Chain verification is delegated to the receipt provider and parsed as a
  provider projection.
- `ReceiptRef.fromSummary` remains invalid because a summary does not carry the
  required hash anchor.
- Invocation builders reject ambiguous delegation plus session-authority
  metadata.
- Authority clients only parse provider-returned metadata values; they do not
  sign, verify, or admit authority locally.

## SPEC Alignment

This slice implements the profile shape described by
`docs/spec/daemon-sdk-requirements-v1.md` sections 5.3, 5.5, 18, and 22. SDK
facades continue to treat receipt URA values as opaque strings returned by
daemon/Axon paths until RFC-007 is resolved.
