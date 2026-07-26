# Architecture

The MCP edge is a projection of canonical AbilityDescriptor facts. It must not own product-specific billing rules.

The root abstraction problem was a third state, `UndeclaredKnownLlm`, which derived cost from `source = "agent:..."` and missing `exec_kind`. That made source metadata double as cost authority. The canonical model is simpler: declared descriptor metadata is authoritative; absence is an explicit undeclared state.
