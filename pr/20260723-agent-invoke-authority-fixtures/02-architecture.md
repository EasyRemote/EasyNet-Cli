# Architecture

Root abstraction problem:

The test fixture used `AxonAbilityCatalog::new_with_runtime(...)`, which hides
authority selection behind ambient test daemon identity. One test also used
`AxonAbilityCatalog::new()` directly to skip setting the dispatch handle.

Refactoring:

- Add a catalog-owned `cfg(test)` runtime-backed explicit Device authority
  constructor.
- Use it in the `agent.invoke` fixture.
- Use the existing metadata-only explicit constructor for the unset-handle test.
