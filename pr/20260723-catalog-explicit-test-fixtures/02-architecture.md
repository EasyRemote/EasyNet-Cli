# Architecture

Root abstraction problem:

Explicit authority catalog fixtures were correct but module-local. That keeps
the construction details coupled to every ability test and invites new
ambient-constructor variants.

Refactoring:

- Add a `cfg(test)` catalog-owned constructor for explicit Device authority
  metadata catalogs.
- Migrate governance metadata and invocation history tests to the shared
  fixture.
- Keep product/runtime constructors separate from test fixture construction.
