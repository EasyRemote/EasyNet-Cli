# Intent

Remove ambient runtime catalog authority construction from MCP bridge tests.

MCP bridge tests need an executable `LocalRuntime` catalog because they validate
dynamic bridge registration and dispatch. The fixture must keep runtime coverage
while binding the catalog to an explicit Device authority root.
