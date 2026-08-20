# API Contract

No public API changes.

Internal test contract:

- Descriptor snapshot tests use `metadata_test_catalog()`.
- Runtime-backed screen tests use `executable_catalog()`, whose construction is
  explicitly Device-authority-bound.
- Direct `AxonAbilityCatalog::new()` and
  `AxonAbilityCatalog::new_with_runtime(...)` calls are not valid inside
  `screen_snapshot` tests.
