# Invariants

1. Agent state reads use `LocalRuntimeStateReadIssuer`.
2. Agent actions continue through `AgentCommandGateway`.
3. `agent.list` is not invoked through `AgentCommandGateway` in production.
4. Tests retain explicit injection seams for both action commands and state
   reads.
5. Public CLI behavior remains unchanged.
