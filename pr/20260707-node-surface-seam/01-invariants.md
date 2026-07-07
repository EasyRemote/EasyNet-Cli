# Invariants

1. Surface is a generic daemon SDK profile; product page rendering belongs to
   downstream backend/frontend products.
2. Every Surface operation requiring daemon dispatch preserves complete
   Invocation carrier context: `caller_ura`, `callee_ura`, `subject_ura`,
   `descriptor_version`, `nonce_base64`, and `causal_context`.
3. Node validates DTO shape and bounds, then delegates carrier construction and
   projection to the injected Surface transport.
4. Node must not build descriptor refs for `pages.*` by string concatenation.
5. Health/status are daemon readiness projections; `SurfaceStatus` is an alias
   of `SurfaceHealth`, not a separate product lifecycle.
6. Public page refs are daemon/backend route facts, not rendering instructions.
7. No non-URA naming and no legacy input aliases are introduced.
