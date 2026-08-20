Goal
====

Align `OwnerKind` authority projection grammar with the canonical
`AuthorityScope` owner marker.

Non-goals
=========

- Do not rename product-facing Hub CLI commands, trust roles, or deployment
  modes in this slice.
- Do not remove product ability names that intentionally contain a `hub.`
  namespace.
- Do not add a compatibility alias from `hub` to `authority`.

Acceptance criteria
===================

- `OwnerKind::RealmAuthority.authority_projection()` returns `authority`.
- `owner_kind_from_projection("authority")` returns `RealmAuthority`.
- `owner_kind_from_projection("hub")` returns `None`.
- SPEC v2 fails if the `hub` owner-kind compatibility parser returns.
