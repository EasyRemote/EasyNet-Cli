# Boundary Proof

Root fork:

- `src/daemon/ability/builtins/agents/list.rs::agent_rows()` receives
  `LocalAgentsFile` and calls `local_agents::lookup_hosted_ura()`.

Owner:

- `src/daemon/persistence/agent_aggregate.rs` owns joined registry plus hosted
  identity read projections.

Accepted path:

- `agent.list` row projection receives `AgentAggregateSnapshot`.
- Rows enumerate registered agents through `registered_agents()`.
- Hosted LLM owner URAs resolve through `hosted_llm_agent_ura()`.

Rejected paths:

- `agent.list` production code must not mention `LocalAgentsFile`.
- `agent.list` production code must not call `lookup_hosted_ura()`.
- `agent.list` production code must not access `snapshot.registry` or
  `snapshot.local_agents` directly.

Effect:

- Architecture convergence: one aggregate object owns the row input contract.
- Product consistency: no output schema or routing behavior changes.
