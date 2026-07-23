# API Contract

No public API changes.

Internal test contract:

- Terminal lifecycle registration tests use `metadata_test_catalog()`.
- Direct `AxonAbilityCatalog::new()` calls are not valid inside terminal
  lifecycle tests.
