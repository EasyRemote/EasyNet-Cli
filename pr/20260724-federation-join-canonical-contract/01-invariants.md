# Invariants

## Semantic invariants

- `federation.join` binds one provisional caller public key to exactly one canonical Device URA.
- `principal_enrollment`, when present, is the only optional principal-binding proof.
- No opaque product pairing secret participates in canonical runtime admission or receipt facts.

## Safety invariants

- Unknown join fields must fail closed at the hub parser/admission boundary.
- The client must not serialize retired fields even when a caller attempts to construct join args programmatically.
- Join receipt verification remains bound to `membership_ura`, `realm`, and `join_receipt_hash`.

## Boundedness invariants

- No extra network round trip or fallback branch is introduced.
- Join remains a single descriptor-bound invoke.
