Semantic invariants
===================

- Every wire envelope caller is a canonical URA.
- A joining device presents its target membership Device URA as both caller and
  subject.
- The new public key is admitted only for `federation.join` and only for the
  exact membership URA whose join payload carries that key.

Safety invariants
=================

- The join route remains restricted to `federation.join`.
- The public key in `public_key_hex` must be the key used to verify the caller
  signature.
- The payload cannot be swapped after proof verification.
- The route/callee realm must match the membership realm.

Boundedness invariants
======================

- Candidate key leases are scoped to one caller URA and are released after
  dispatch.
- No fallback resolver may accept arbitrary unknown callers.
