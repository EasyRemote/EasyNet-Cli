# API Contract

No public API changes.

Internal test contract:

- A2A bridge tests use `metadata_test_catalog()`.
- Direct `AxonAbilityCatalog::new()` calls are not valid inside
  `a2a/bridge.rs` tests.
