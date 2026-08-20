# Invariants

1. Boot-time `agent.discover` registration must not inject a direct `agent_registry::load_agents()` provider.
2. Boot-time `a2a.bridge.list_skills` registration must not inject a direct `agent_registry::load_agents()` provider.
3. Both providers must load `AgentAggregateRepository::load_snapshot()` and consume `registered_agent_registry_projection()`.
4. Handler public contracts remain registry-shaped until the larger discovery implementation is migrated, but the registry projection must come from the aggregate.
5. The convergence script must fail if either boot registration block returns to a raw registry provider.
