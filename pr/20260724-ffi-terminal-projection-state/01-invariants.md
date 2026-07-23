# Invariants

1. Stream/bidi reader control flow must not infer terminality by indexing callback JSON.
2. Canonical lifecycle terminality remains receipt-bound:
   - stream terminality is true only when a verified terminal receipt exists;
   - bidi terminality is true only when a verified terminal receipt payload exists.
3. Transport failures and callback backpressure are terminal for the local carrier only, never canonical runtime terminality.
4. Missing, malformed, or renamed public JSON fields must not silently change reader lifecycle behavior.
5. Public callback JSON shape remains compatible.

