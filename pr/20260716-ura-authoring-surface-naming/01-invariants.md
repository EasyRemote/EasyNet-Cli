# Invariants

1. EasyNet/Axon routable identities are URAs, not URIs.
2. Skill metadata must not publish `axon-resource-uri`; new authored resources
   use `axon-resource-ura`.
3. Page authoring examples must present `project_ura`, not `project_uri`.
4. The identity guide may show raw string construction only as a forbidden
   example, and the variable name in that example must not reintroduce URI-era
   terminology.
5. Address values stay byte-for-byte compatible where this slice only renames
   authoring labels.
