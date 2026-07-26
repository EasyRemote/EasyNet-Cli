# Invariants

1. Authority metadata is a signed admission fact, not an extensible product envelope.
2. Unknown fields at this boundary are malformed input and must fail before admission projection.
3. Cryptographic verification remains owned by admission; this slice only hardens canonical shape validation.
4. Runtime projection consumes only canonical authority payloads; it must not repair, alias, or ignore retired carriers.
5. No public API field names change.
6. URA terminology remains the only identity/address terminology in new code and tests.
