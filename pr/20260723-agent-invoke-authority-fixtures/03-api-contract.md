# API Contract

No public API changes.

Internal test contract:

- Runtime-backed tests use
  `AxonAbilityCatalog::new_test_runtime_for_device_authority(...)`.
- Metadata-only tests use
  `AxonAbilityCatalog::new_test_metadata_for_device_authority(...)`.
- `AxonAbilityCatalog::new()` and `new_with_runtime(...)` are not valid
  `agent.invoke` fixtures.
