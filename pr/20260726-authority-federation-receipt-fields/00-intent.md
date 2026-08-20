Goal
====

Retire Hub-named federation ability receipt facts from the canonical runtime
receipt contract and replace them with Authority-published ability vocabulary.

Non-goals
=========

- Do not keep serde aliases for the retired Hub field names.
- Do not change invocation envelope shape or ability names in this slice.
- Do not rename canonical Hub URA transport concepts unrelated to ability
  publication facts.

Acceptance criteria
===================

- Join receipts use `authority_published_abilities` and
  `authority_abilities_revision`.
- Heartbeat receipts use `authority_abilities_diff`.
- DTO types are `AuthorityAbilityEntry` and `AuthorityAbilitiesDiff`.
- Producer and consumer tests use the Authority field names.
- SPEC v2 rejects the retired Hub receipt fact names.
