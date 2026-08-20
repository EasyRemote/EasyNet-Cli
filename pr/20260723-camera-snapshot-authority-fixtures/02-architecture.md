# Architecture

Root abstraction problem:

Camera media tests mixed direct `AxonAbilityCatalog::new()` and
`AxonAbilityCatalog::new_with_runtime(...)` construction. That allowed
process-local test daemon identity to choose the authority context instead of
the tests declaring the Device-hosted media surface explicitly.

Refactoring:

- Add module-local metadata-only and runtime-backed catalog helpers.
- Bind both helpers to one explicit Device authority root.
- Keep the existing `executable_catalog()` semantic role while removing its
  ambient constructor dependency.
- Keep production camera backend, stream handling, and recording lifecycle
  behavior unchanged.
