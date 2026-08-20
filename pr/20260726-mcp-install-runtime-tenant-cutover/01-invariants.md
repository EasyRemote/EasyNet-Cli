# Invariants

1. MCP install is a persistent client configuration mutation and must not bind
   to synthesized runtime state.
2. Runtime tenant is a lifecycle fact. Absence means "not bound", not
   "default".
3. A blank explicit tenant is invalid and must fail before config write.
4. Installed server args remain exactly `easynet mcp serve --tenant <tenant>`
   plus optional `--agent`.
