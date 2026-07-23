# API Contract

No public API changes.

Internal test contract:

- Media metadata and stub registration tests use `metadata_test_catalog()`.
- Direct `AxonAbilityCatalog::new()` calls are not valid inside
  `media::abilities` tests.
