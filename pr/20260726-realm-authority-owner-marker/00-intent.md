Goal
====

Retire the `hub` owner-plane marker from runtime authority binding scope.

Non-goals
=========

- Do not rename product-facing CLI commands or deployment mode strings in this
  slice.
- Do not change Authority URA construction.
- Do not add compatibility parsing for retired `hub` scope markers.

Acceptance criteria
===================

- `AuthorityScope` canonicalizes the realm Authority owner plane as
  `authority`.
- `hub` is rejected as a retired runtime authority owner projection.
- SPEC v2 fails if `hub -> RealmAuthority` compatibility parsing returns.
