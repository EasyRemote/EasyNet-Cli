# Invariants

1. Receipt proof facts are canonical runtime evidence, not product
   compatibility data.
2. `opaque` may describe SDK-owned payload treatment, but it is not a receipt
   entity URA profile.
3. `axon-legacy-v1` must not be accepted by any SDK receipt proof-fact
   validator.
4. The public receipt APIs remain source-compatible; the internal validator
   becomes stricter.
5. All languages must expose the same capability state for receipt proof facts:
   provider-backed validation over the canonical strict profile.
