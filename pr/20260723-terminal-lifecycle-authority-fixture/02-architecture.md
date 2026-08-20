# Architecture

Root abstraction problem:

The terminal lifecycle registration test used `AxonAbilityCatalog::new()`,
allowing process-local test daemon identity to choose the authority context.
That hides the Device authority under which terminal lifecycle abilities are
registered.

Refactoring:

- Add a module-local metadata-only catalog helper.
- Bind the helper to one explicit Device authority root.
- Migrate the registration test to the helper.
- Keep production terminal lifecycle registration and PTY service behavior
  unchanged.
