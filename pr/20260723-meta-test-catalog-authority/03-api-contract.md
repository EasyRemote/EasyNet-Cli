# API Contract

No public API changes.

Internal test contract:

- Metadata-only tests use `metadata_test_catalog()`.
- Runtime-backed metadata tests use `runtime_metadata_test_catalog(...)`.
- Direct ambient catalogue constructors are not valid fixtures for metadata
  tests.
