# Canonical Runtime Convergence V2 - API Contract

## Public Boundary

The canonical public runtime boundary remains descriptor-bound invocation:

`caller, callee, ability, subject, nonce, causal_context, args -> receipt`

No compatibility adapter may mint missing tuple fields, signer material, or
receipt proof facts inside the canonical domain.

## Descriptor Projection Contract

`GovernedSchemaProjection` is an internal daemon descriptor projection object.
It is consumed only to produce the governed JSON summary used for schema hash
calculation.

The projection is intentionally value-complete. Adding a new descriptor proof
input must add a named field to the projection object and must update the hash
verification tests. Callers must not pass ad hoc positional arguments for
descriptor hash material.
