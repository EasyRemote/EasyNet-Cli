# API Contract

No public API changes.

Internal test contract:

- `registry_with_joined_device_home()` means isolated HOME plus explicit joined
  Device credentials.
- `registry_with_temp_home()` means isolated empty HOME and must not be used by
  tests that mint local filesystem ResourceRefs or operate on the local device
  row.
- Direct calls to `fs_ref()` must happen while a joined Device fixture guard is
  alive.
