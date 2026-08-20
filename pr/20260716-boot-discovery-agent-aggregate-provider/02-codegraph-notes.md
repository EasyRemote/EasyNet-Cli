# CodeGraph Notes

## Static Graph Evidence

- `src/daemon/ability/catalog/build.rs::build_registry_with_services_result_inner` registers `agent.discover` with a closure that calls `agent_registry::load_agents()`.
- The same function registers `a2a_bridge_ability::register` with another direct `agent_registry::load_agents()` closure.
- `src/daemon/ability/builtins/agents/chat.rs` already uses `AgentAggregateRepository::load_snapshot().registered_agent_registry_projection()` for hot-added Agent discover handlers.
- `src/daemon/ability/builtins/agents/discover.rs::LocalAgentAbilityOwners::load` independently consumes the hosted identity aggregate snapshot, so the remaining split source is the injected registered-Agent provider.

## Boundary Decision

Keep `AgentRegistryProvider` as an adapter contract for now, but require production boot providers to project it from `AgentAggregateSnapshot`.
