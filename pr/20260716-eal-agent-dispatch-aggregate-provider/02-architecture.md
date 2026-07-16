# Architecture

## Boundary

`src/daemon/persistence/agent_aggregate.rs` owns Agent read projections. `src/eal/interpreter/dispatch.rs` is an execution consumer and must not read persistence files directly.

## Layering

- Persistence/domain: `AgentAggregateRepository` reads durable registry and hosted identity state.
- Persistence/domain: `AgentAggregateSnapshot::registered_agent_registry_projection` adapts registered Agent state for legacy provider signatures.
- EAL execution: `AgentAwareDispatcher` stores the registered Agent projection and dispatches through daemon Invocation.

## Expected Effect

This slice removes one execution-path registry-only read. The concrete product effect is consistent EAL agent dispatch startup behavior with other Agent read surfaces: registry state comes from the same aggregate owner used by chat, teach, mission, route, and admission paths.
