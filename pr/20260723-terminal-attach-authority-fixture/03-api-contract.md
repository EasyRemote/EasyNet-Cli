# API Contract

No public API changes.

Internal test contract:

- Terminal attach registration tests use `metadata_test_catalog()`.
- Direct `AxonAbilityCatalog::new()` calls are not valid inside terminal attach
  tests.
