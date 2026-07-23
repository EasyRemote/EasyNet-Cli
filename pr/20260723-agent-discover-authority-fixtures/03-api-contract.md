# API Contract

No public API changes.

Internal test contract:

- Runtime-backed `agent.discover` tests use `runtime_test_catalog()`.
- Metadata-only `agent.discover` tests use `metadata_test_catalog()`.
- Direct use of `AxonAbilityCatalog::new()` and
  `AxonAbilityCatalog::new_with_runtime(...)` is not valid inside
  `agent.discover` tests.
