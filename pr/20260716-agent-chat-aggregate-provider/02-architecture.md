# Architecture

## Boundary

`src/daemon/persistence/agent_aggregate.rs` owns Agent read projections. `src/daemon/ability/builtins/agents/chat.rs` is an invocation surface consumer and must not assemble persistence reads itself.

## Layering

- Persistence/domain: `AgentAggregateRepository` loads registry and hosted identity state and returns an immutable snapshot.
- Persistence/domain: `AgentAggregateSnapshot` exposes registered Agent projections needed by ability consumers.
- Ability surface: chat hot-added discover/invoke handlers build provider closures from aggregate projections.
- Ability surface: chat peer-skill enumeration consumes the same projection when constructing advisory tool context.

## Expected Effect

This slice reduces Agent source-of-truth splitting on the chat invocation path. The concrete product effect is consistent hot-add behavior: discover, invoke, and chat tool hints all see registered Agent state through the same aggregate read owner.
