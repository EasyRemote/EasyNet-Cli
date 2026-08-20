Daemon Principal route binding convergence

Goal

Remove the remaining handwritten PrincipalLifecycle ability literals from the
daemon runtime boundary. The same provider route manifest that now feeds Go,
Python, and the Rust CLI should also feed daemon conformance and admission
constants, while preserving existing internal constant paths.

Expected effect

- One provider-owned route manifest drives SDK, CLI, conformance, and daemon
  PrincipalLifecycle admission route names.
- Existing daemon constants such as `ABILITY_PRINCIPAL_CREATE` remain available
  to callers, but become aliases of generated route bindings.
- PrincipalLifecycle state-machine behavior, descriptor contracts, receipt
  semantics, and public APIs remain unchanged.
