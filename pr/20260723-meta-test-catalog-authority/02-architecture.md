# Architecture

Root abstraction problem:

Metadata tests were using the production convenience constructor as a fixture.
That couples catalogue behavior tests to host pairing state and leaves product
validation dependent on local credentials.

Refactoring:

- Add local metadata-test catalogue fixture builders.
- Replace `AxonAbilityCatalog::new()` with explicit metadata-only authority
  construction.
- Replace live test catalogues with explicit
  `new_with_runtime_and_authority_context`.
