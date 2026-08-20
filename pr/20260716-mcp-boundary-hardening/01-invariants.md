# Invariants

- `McpToolRouteTable` remains the single descriptor-to-MCP route projection
  owner for bridge handlers and stdio MCP surfaces.
- `mcp.bridge.list_tools` advertises descriptors whose `CallMode` is `Rpc` only.
- `mcp.bridge.call_tool` resolves only the advertised MCP tool names in the same
  route table used by list projection; stream and bidi descriptors cannot be
  invoked through this unary surface.
- MCP stdio line ingestion returns `TooLong` once the declared frame limit is
  exceeded, including EOF without a trailing newline.
- The bounded reader never retains more bytes than the declared per-frame limit.
- This slice adds executable gates around existing boundaries; it does not add
  compatibility fallbacks or product semantics to the canonical SDK.
