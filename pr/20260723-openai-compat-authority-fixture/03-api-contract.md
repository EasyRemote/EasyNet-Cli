# API Contract

No public API changes.

Internal test contract:

- OpenAI compatibility tests use `metadata_test_catalog()`.
- Direct `AxonAbilityCatalog::new()` calls are not valid inside
  `openai_compat.rs` tests.
