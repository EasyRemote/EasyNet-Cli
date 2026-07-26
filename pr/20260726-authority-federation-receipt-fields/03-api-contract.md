API contract
============

Join receipt
------------

- `authority_published_abilities: AuthorityAbilityEntry[]`
- `authority_abilities_revision: u64`

Heartbeat receipt
-----------------

- `authority_abilities_diff: AuthorityAbilitiesDiff`

Failure contract
----------------

- Old Hub field names are rejected as missing required facts.
- Descriptor rows still fail closed when the canonical descriptor cannot be
  parsed or lacks a descriptor reference.
