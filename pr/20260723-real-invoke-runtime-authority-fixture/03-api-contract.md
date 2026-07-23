# API Contract

No public API changes.

Internal test contract:

- Local real-invoke tests use `runtime_attached_catalog()`.
- `runtime_attached_catalog()` binds runtime execution to an explicit combined
  authority context rooted at the fixture Device URA.
- `runtime_attached_catalog_for_realm()` remains the custom realm fixture for
  tests that need declared hosted Agent authority roots.
