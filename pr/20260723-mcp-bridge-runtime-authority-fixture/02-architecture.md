# Architecture

Root abstraction problem:

The MCP bridge executable test catalog used `AxonAbilityCatalog::new_with_runtime`
directly. That attaches `LocalRuntime` while leaving authority selection to the
constructor's ambient test default.

Refactoring:

- Keep the fixture executable and runtime-backed.
- Bind the fixture to one explicit Device authority root.
- Reuse the canonical `new_test_runtime_for_device_authority` constructor.
- Keep production MCP bridge registration behavior unchanged.
