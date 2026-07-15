# Invariants

1. MCP tool publication is derived from descriptor call mode, not ability name.
2. Only `CallMode::Rpc` descriptors are MCP-callable in the current provider.
3. Stream and bidi descriptors cannot be resolved by `tools/call` unless a real
   stream/bidi MCP provider is added.
4. `tools/list` and `tools/call` share the same route table snapshot.
5. No unary fallback is introduced for stream or bidi abilities.
