# Architecture

Root abstraction problem:

Ability management tests still mix ambient catalog constructors with fixtures
that should describe a specific authority mode. That hides whether the test is
checking metadata/control-plane registration or canonical runtime execution.

Refactoring:

- Introduce explicit module-local fixture constructors in `ops.rs`.
- Bind metadata-only fixtures to one Device authority root.
- Bind executable fixtures to one Device authority root and an explicit
  LocalRuntime when needed.
- Keep production ability management registration behavior unchanged.
