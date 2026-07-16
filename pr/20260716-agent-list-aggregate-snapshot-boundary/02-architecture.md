# Agent List Aggregate Snapshot Architecture

## Owner Boundary

`AgentAggregateRepository` is the read-side owner for paired hosted-Agent state:

- `AgentRegistry` from `agents.json`,
- `LocalAgentsFile` from `local-agents.json`.

The returned `AgentAggregateSnapshot` is an immutable per-call value.

## Layering

- Persistence owns file reads and aggregate construction.
- `agent.list` owns only public response projection.
- Tests may inject snapshots at the ability boundary.
- Lifecycle mutation ownership remains in `AgentLifecycleProjectionStore`.

## Migration Rule

Production read surfaces that need both registry and hosted identities should migrate to `AgentAggregateRepository::load_snapshot()` instead of independently loading persistence files.
