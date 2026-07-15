# Intent: MCP stdio bounded-reader gate

## Goal

Close the A24 bounded-behavior regression class for MCP stdio by making the
existing bounded reader architecture executable in the architecture convergence
gate.

The concrete use case is a malformed or hostile MCP peer that sends a newline-
free line or an oversized `Content-Length` frame. The daemon must reject the
frame before retaining unbounded bytes or allocating the advertised body.

## Scope

- Guard the daemon MCP stdio transport owner.
- Reject reintroduction of `read_line`-style unbounded line reads in the MCP
  stdio transport owner.
- Require explicit bounded line and content-length frame limits.
- Add a negative fixture proving the gate catches the old architecture shape.

## Non-goals

- No public MCP protocol change.
- No SDK surface change.
- No behavior refactor beyond preserving the already converged bounded reader
  owner.
