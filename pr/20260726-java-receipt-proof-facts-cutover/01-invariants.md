# Invariants

1. Receipt proof facts are signed runtime evidence, not optional DTO enrichment.
2. Missing descriptor/runtime proof facts must fail before any canonical receipt projection is accepted.
3. Identity proof profile must be exactly `axon-strict-v2`.
4. Java, Go, and Python SDKs must converge on the same mandatory proof-fact matrix.
5. No language SDK may keep a local legacy/opaque proof profile compatibility path.
