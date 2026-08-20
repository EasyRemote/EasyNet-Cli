# Decisions

1. Match the spec's generic authority DTO semantics in Node.
2. Keep Node authority minting transport-injected and seam-level.
3. Do not modify the existing provider-backed authority conformance case to fit
   Node.
4. Validate ambiguous authority metadata in InvocationBuilder so admission
   metadata cannot silently carry two authority families.
5. Override the current Go/Python product-shaped `SessionAuthority` DTO for
   this Node seam and use the spec-shaped issuer/subject/audience model. The
   Go/Python shape is now treated as follow-up convergence work, not a pattern
   to replicate in P1 facades.
