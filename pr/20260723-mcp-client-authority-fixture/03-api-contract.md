# API Contract

No public API changes.

Internal test contract:

- MCP client tests use `metadata_test_catalog()`.
- Direct `AxonAbilityCatalog::new()` calls are not valid inside MCP client
  tests.
