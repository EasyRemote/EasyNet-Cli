# API Contract

No public API changes.

Internal test contract:

- `AxonAbilityCatalog::new_test_metadata_for_device_authority(device_ura)` builds
  a metadata-only catalog for a declared Device authority.
- This helper is available only under `cfg(test)`.
