# API Contract

No public API changes.

Internal test contract:

- File transfer registration tests use `metadata_test_catalog()`.
- Direct `AxonAbilityCatalog::new()` calls are not valid inside file transfer
  tests.
