# Hot Agent Authority Aggregate Snapshot Architecture

## Owner Boundary

`AgentAggregateRepository` owns paired persistence loading. `AgentAggregateSnapshot` owns read-only predicates and identity lookup over that paired state.

## Layering

- Persistence layer: loads and exposes immutable aggregate state.
- Dispatch authority inventory: maps aggregate snapshot facts into authority-domain errors and state transitions.
- Lifecycle mutation layer remains the writer for durable Agent state.

## Migration Rule

Authority proof paths must not load `agent_registry` and `local_agents` directly. They must consume a snapshot and map source-preserving load errors into authority-domain errors.
