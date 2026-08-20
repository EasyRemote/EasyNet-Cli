# API Contract

No public API changes.

Internal test contract:

- Use `invocation_history_test_catalog()` for metadata-only registration tests.
- Runtime-backed tests must pass an explicit `AbilityAuthorityContext`.
- `AxonAbilityCatalog::new()` is not a valid invocation-history governance
  fixture.
