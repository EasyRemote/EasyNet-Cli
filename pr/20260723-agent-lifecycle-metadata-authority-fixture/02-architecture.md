# Architecture

Root abstraction problem:

`registration_makes_lifecycle_abilities_dispatchable` used
`AxonAbilityCatalog::new()`. For this metadata-only test the constructor works,
but it hides the authority root behind the ambient test environment.

Refactoring:

- Keep registration coverage metadata-only because the test does not execute
  handlers.
- Bind the catalog to one explicit Device authority root.
- Reuse `new_test_metadata_for_device_authority`, the canonical metadata test
  seam already used by other builtins.
- Leave lifecycle runtime fixtures and production registration untouched.
