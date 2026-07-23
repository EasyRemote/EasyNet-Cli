# Architecture

Root abstraction problem:

A2A bridge tests used `AxonAbilityCatalog::new()` directly. That made the
authority context ambient even though the test only needs deterministic
metadata/control-plane registration.

Refactoring:

- Add a module-local metadata-only catalog helper.
- Bind the helper to one explicit Device authority root.
- Migrate the A2A bridge registration test to the helper.
- Keep production A2A bridge registration behavior unchanged.
