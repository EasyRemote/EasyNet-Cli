# Architecture

Root abstraction problem:

The file transfer registration test used `AxonAbilityCatalog::new()`, allowing
ambient test daemon identity to choose the authority context. That hides the
Device authority under which file transfer abilities are registered.

Refactoring:

- Add a module-local metadata-only catalog helper.
- Bind the helper to one explicit Device authority root.
- Migrate the registration test to the helper.
- Keep production file transfer registration and handler behavior unchanged.
