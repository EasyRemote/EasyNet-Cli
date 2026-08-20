# Architecture

Root abstraction problem:

Screen media tests mixed direct `AxonAbilityCatalog::new()` and
`AxonAbilityCatalog::new_with_runtime(...)` construction. That allowed
process-local test daemon identity to choose the authority context instead of
the test declaring the Device-hosted media surface explicitly.

Refactoring:

- Add module-local metadata-only and runtime-backed catalog helpers.
- Bind both helpers to one explicit Device authority root.
- Keep the existing `executable_catalog()` semantic role while removing its
  ambient constructor dependency.
- Keep production screen backend and registration behavior unchanged.
