Execution checklist
===================

- [x] Remove `impl Default for AbilityAuthorityContext`.
- [x] Remove ambient `AxonAbilityCatalog::new()` and `#[derive(Default)]`.
- [x] Migrate tests to explicit device-authority catalog fixtures.
- [x] Add a SPEC v2 static guard against reintroducing these constructors.
- [x] Run targeted Rust compile/test coverage for ability dispatch/catalog.
- [x] Run fmt, diff check, and convergence gates.
