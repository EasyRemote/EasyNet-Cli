Goal
====

Retire enum-level defaults for descriptor visibility and scope policy.

Non-goals
=========

- Do not change authored descriptor policy semantics.
- Do not change `AbilityDescriptor::new` constructor defaults in this
  iteration.
- Do not change TOML wire syntax or public descriptor JSON.

Acceptance criteria
===================

- `Visibility` no longer implements `Default`.
- `ScopeRule` no longer implements `Default`.
- Descriptor parser/builders continue to select policy states explicitly.
- SPEC v2 rejects reintroducing default policy enum states.
