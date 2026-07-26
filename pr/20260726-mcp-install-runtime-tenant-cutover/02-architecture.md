# Architecture

`src/cli/mcp/install.rs` owns the operator-facing projection into Claude/Codex
MCP client config. It depends on runtime session state only to reuse an existing
tenant binding. It is not allowed to mint one.

The root abstraction problem was treating missing runtime session state as a
configuration default. That converts an unbound lifecycle state into a durable
client config, producing a false-success installation.
