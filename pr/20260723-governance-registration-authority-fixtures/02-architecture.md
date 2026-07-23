# Architecture

Root abstraction problem:

Small governance tests used the production convenience catalog constructor as a
fixture. That constructor is a poor test boundary because it can couple
registration assertions to local daemon authority state.

Refactoring:

- Use the catalog-owned explicit Device authority fixture.
- Keep each ability module focused on ability semantics.
- Do not add local fixture wrappers for one-off registration tests.
