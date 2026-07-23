# Architecture

Root abstraction problem:

`mic.subscribe` tests directly used `AxonAbilityCatalog::new()` and
`AxonAbilityCatalog::new_with_runtime(...)`. Those constructors hide the
authority context behind process-local test daemon identity, which weakens the
resource/media stream boundary being tested.

Refactoring:

- Add module-local metadata-only and runtime-backed catalog helpers.
- Bind both helpers to one explicit Device authority root.
- Migrate all `mic.subscribe` test catalog construction to those helpers.
- Keep production media registration and stream handling unchanged.
