# Architecture

Root abstraction problem:

Media metadata tests used `AxonAbilityCatalog::new()`, allowing process-local
test daemon identity to choose the authority context. That hides the Device
authority under which physical media ability metadata is registered.

Refactoring:

- Add a module-local metadata-only catalog helper.
- Bind the helper to one explicit Device authority root.
- Migrate all media metadata/stub registration tests to the helper.
- Keep production media ability table and registration behavior unchanged.
