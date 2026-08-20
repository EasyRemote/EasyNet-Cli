# API Contract

No public API changes.

Internal test contract:

- Terminal I/O registration tests use `metadata_test_catalog()`.
- Direct `AxonAbilityCatalog::new()` calls are not valid inside terminal I/O
  tests.
