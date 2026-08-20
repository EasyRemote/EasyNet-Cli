# API Contract

No public API changes.

Internal test contract:

- MCP bridge executable tests use `executable_test_catalog()`.
- `executable_test_catalog()` must bind LocalRuntime to an explicit Device
  authority root.
- Direct `AxonAbilityCatalog::new_with_runtime()` calls are not valid inside
  `mcp/bridge.rs` tests.
