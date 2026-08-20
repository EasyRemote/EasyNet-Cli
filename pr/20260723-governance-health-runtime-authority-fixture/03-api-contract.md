# API Contract

No public API changes.

Internal test contract:

- `registration_makes_ability_dispatchable` remains a LocalRuntime-backed smoke
  test.
- The fixture must use an explicit Device authority root.
- Direct `AxonAbilityCatalog::new_with_runtime()` is not valid inside
  `governance/health.rs` tests.
