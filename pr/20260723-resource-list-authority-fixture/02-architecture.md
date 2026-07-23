# Architecture

Root abstraction problem:

The registration test used the production convenience catalog constructor even
though it only asserts local Device ability registration. That couples a
resource discovery test to ambient daemon authority state.

Refactoring:

- Replace `AxonAbilityCatalog::new()` with the catalog-owned explicit Device
  authority fixture.
- Do not add a local helper because there is only one catalog construction in
  this module.
