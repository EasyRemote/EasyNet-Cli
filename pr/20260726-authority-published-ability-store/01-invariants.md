Invariants
==========

- The store remains an in-memory monotonic read model keyed by canonical
  `AbilityDescriptor` identity.
- Applying an older revision never overwrites a newer snapshot.
- Invalid descriptors still fail closed at store ingress.
- Local catalogue rows and Authority-published rows remain merged only at
  `meta.list_abilities(scope="realm")`.
- Wire DTO names remain unchanged until a dedicated federation protocol
  migration exists.
