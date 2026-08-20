Goal
====

Retire Hub vocabulary from realm-scope ability catalogue projection metadata.

Non-goals
=========

- Do not rename federation wire fields such as `hub_abilities_diff` in this
  slice.
- Do not rename the existing read-model module file.
- Do not change descriptor identity, ability URAs, or routing behavior.

Acceptance criteria
===================

- `meta.list_abilities(scope="realm")` annotates broadcast rows with
  `authority:broadcast`.
- User-facing schema text describes realm Authority-published abilities.
- SPEC v2 rejects reintroducing `hub:broadcast` in `meta.list_abilities`.
