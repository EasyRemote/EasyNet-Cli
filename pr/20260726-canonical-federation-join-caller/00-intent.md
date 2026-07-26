Goal
====

Retire the legacy non-URA federation-join pseudo-caller model and make the join
envelope use the canonical membership Device URA as caller and subject.

Non-goals
=========

- Do not weaken Axon canonical envelope admission to accept non-URA callers.
- Do not add a compatibility alias for the retired pseudo-caller model.
- Do not change the public `federation.join` ability name.
- Do not remove candidate key leasing; first join still needs a bounded key
  resolver seam before the membership key is persisted.

Acceptance criteria
===================

- `federation.join` test helpers construct caller/callee/subject as canonical
  URAs.
- The bootstrap proof binds membership URA, public key, route, realm, and args.
- Candidate key leasing is keyed by canonical membership URA, not a
  non-canonical pseudo-identity.
- The previously failing federation join service tests pass.
