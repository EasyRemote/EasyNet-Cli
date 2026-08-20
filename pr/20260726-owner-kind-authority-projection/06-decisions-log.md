Decisions log
=============

2026-07-26
----------

- Treat `hub` owner projection parsing in `OwnerKind` as a compatibility layer
  after `AuthorityScope` moved to `authority`.
- Keep product-facing Hub naming out of scope; this slice only changes runtime
  owner grammar.
- Update descriptor/control-plane comments because comments are part of the
  architecture contract for future contributors and SDK consumers.
