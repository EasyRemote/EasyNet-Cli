# Architecture

Root abstraction problem:

`agent.discover` tests used `AxonAbilityCatalog::new()` and
`AxonAbilityCatalog::new_with_runtime(...)`, which let test process state pick
the authority context. That hides whether the ability surface is hosted by the
intended Device authority.

Refactoring:

- Add discover-test-local helpers that build metadata-only and runtime-backed
  catalogs with an explicit Device authority root.
- Migrate all `agent.discover` test registry construction to those helpers.
- Keep production registry constructors unchanged.
