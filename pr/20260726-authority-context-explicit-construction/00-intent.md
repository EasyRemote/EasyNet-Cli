Goal
====

Remove the ambient `Default` implementation for `AbilityAuthorityContext`.

Non-goals
=========

- Do not change daemon boot authority discovery.
- Do not change descriptor shape, owner URA syntax, or LocalRuntime ability keys.
- Do not add a compatibility constructor or fallback authority source.

Acceptance criteria
===================

- `AbilityAuthorityContext` no longer implements `Default`.
- `AxonAbilityCatalog` no longer derives `Default` or exposes an ambient
  metadata-only `new()` constructor.
- All authority context construction remains explicit at call sites.
- SPEC v2 rejects reintroducing an ambient default constructor for this type.
