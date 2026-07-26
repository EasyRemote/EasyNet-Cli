Goal
====

Retire the internal `hub_published_abilities` read-model vocabulary from the
runtime ability catalogue path and make the store name match the canonical
Authority-published ability model.

Non-goals
=========

- Do not rename federation wire fields such as `hub_published_abilities` or
  `hub_abilities_diff` in this slice.
- Do not change federation receipt JSON compatibility.
- Do not change descriptor identity, ability URAs, routing behavior, or
  catalogue response shape beyond internal source ownership.

Acceptance criteria
===================

- Runtime code imports and passes an `AuthorityPublishedAbilityStore`.
- The read-model module is named `authority_published_abilities`.
- SPEC v2 requires the canonical store/module/type and rejects reintroducing
  the old internal store name.
- Existing federation join/heartbeat catalogue tests keep passing.
