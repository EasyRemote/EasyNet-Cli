# API Contract

No public API changes.

Internal test contract:

- Agent lifecycle registration tests use explicit metadata authority fixtures.
- Direct `AxonAbilityCatalog::new()` is not valid inside
  `agents/lifecycle.rs` tests.
- Runtime-backed lifecycle fixtures continue to use explicit authority contexts.
