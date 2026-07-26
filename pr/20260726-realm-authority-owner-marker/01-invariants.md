Invariants
==========

- Runtime authority binding scopes use canonical runtime vocabulary, not
  product deployment vocabulary.
- A realm Authority scope marker must be stable, explicit, and parseable by one
  owner-projection state machine.
- Retired `hub` owner projection data must fail closed; it must not be silently
  repaired to `authority`.
- Public product labels may remain outside this internal authority binding
  model until their own migration slice.
