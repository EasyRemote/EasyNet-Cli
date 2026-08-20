# API Contract

No public API changes.

Internal test contract:

- Ability management tests do not call `AxonAbilityCatalog::new()` directly.
- Ability management tests do not call `AxonAbilityCatalog::new_with_runtime()`
  directly when a Device authority fixture is semantically required.
- Test helper names must state metadata-only versus executable runtime intent.
