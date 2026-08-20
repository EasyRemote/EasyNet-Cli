Architecture
============

Layering
--------

- Federation wire contracts continue to deserialize existing receipt payloads.
- The daemon read model owns the canonical runtime concept:
  `AuthorityPublishedAbilityStore`.
- `meta.list_abilities` depends on the read model through the canonical store
  name, not through Hub vocabulary.

Boundary proof
--------------

This slice removes internal runtime naming debt without changing wire
compatibility. The old name is not retained as a type alias because that would
preserve a compatibility layer inside the runtime model.
