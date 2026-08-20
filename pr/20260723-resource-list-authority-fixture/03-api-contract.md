# API Contract

No public API changes.

Internal test contract:

- `meta.list_resources` registration tests use
  `AxonAbilityCatalog::new_test_metadata_for_device_authority(...)`.
- `AxonAbilityCatalog::new()` is not a valid fixture for this module.
