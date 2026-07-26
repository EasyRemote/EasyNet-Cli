Target boundary:

```text
C ABI runtime_resolve_descriptor_ref
  -> parse UTF-8 / handle lookup
  -> daemon::axon_bridge::runtime_descriptor_provider
  -> runtime descriptor catalog authority
  -> typed resolution JSON or typed failure
```

Module ownership:
- `src/ffi/invocation/mod.rs` owns C ABI memory, handle lookup, and error projection only.
- `src/daemon/axon_bridge/runtime_descriptor_provider.rs` owns descriptor request validation, provider selection, owner resolution, catalog materialization, and row selection.
- `src/daemon/axon_bridge/descriptor_ref.rs` continues to own pure descriptor reference normalization.

Deletion list:
- Remove FFI-local `RuntimeDescriptorCatalog`.
- Remove FFI-local catalog construction helpers.
- Remove FFI-local descriptor row resolution helpers.
