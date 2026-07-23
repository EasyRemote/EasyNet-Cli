# Architecture

Root abstraction problem:

MCP client tests used `AxonAbilityCatalog::new()` directly. That makes the
authority context ambient and hides whether the integration surface is
registered under the Device authority model.

Refactoring:

- Add a module-local metadata-only catalog helper.
- Bind the helper to one explicit Device authority root.
- Migrate MCP client tests to the helper.
- Keep production MCP client registration behavior unchanged.
