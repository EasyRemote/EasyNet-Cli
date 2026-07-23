# API Contract

No public API changes.

Internal test contract:

- Metadata tests use `metadata_test_catalog()`.
- Stream execution tests use `runtime_test_catalog()`.
- Direct `AxonAbilityCatalog::new()` and
  `AxonAbilityCatalog::new_with_runtime(...)` calls are not valid inside
  `mic.subscribe` tests.
