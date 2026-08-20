# Architecture

Root abstraction problem:

OpenAI compatibility tests used `AxonAbilityCatalog::new()` directly. That made
the authority context ambient even though the tests only need a deterministic
metadata/control-plane catalog.

Refactoring:

- Add a module-local metadata-only catalog helper.
- Bind the helper to one explicit authority root.
- Migrate OpenAI compatibility tests to the helper.
- Keep production registration and adapter behavior unchanged.
