# Architecture

- `src/daemon/ability/builtins/device_control/files.rs` owns host filesystem
  system abilities.
- `src/daemon/ability/builtins/resources/files_store` owns the user
  content-addressed blob store.
- `src/daemon/ability/builtins/integrations/openai_compat.rs` is a Device-owned
  compatibility facade; it invokes the user-owned files surface instead of
  owning blob state itself.
- `src/daemon/ability/catalog/build.rs` declares daemon-native executor roots
  before catalog construction.
