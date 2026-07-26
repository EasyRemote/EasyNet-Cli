# Invariants

- Cost metadata is catalog evidence, not a runtime guess.
- Undeclared cost is explicit: `cost_kind = "unknown"`, `cost_label = "cost not declared"`.
- Declared cost metadata preserves labels when present and derives labels only from declared cost kind when omitted.
- MCP clients receive the same field names regardless of declaration state.
