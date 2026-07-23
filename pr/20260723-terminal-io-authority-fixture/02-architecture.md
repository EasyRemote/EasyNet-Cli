# Architecture

Root abstraction problem:

The terminal I/O registration test used `AxonAbilityCatalog::new()`, allowing
process-local test daemon identity to choose the authority context. That hides
the Device authority under which terminal I/O abilities are registered.

Refactoring:

- Add a module-local metadata-only catalog helper.
- Bind the helper to one explicit Device authority root.
- Migrate the registration test to the helper.
- Keep production terminal I/O registration and PTY I/O behavior unchanged.
