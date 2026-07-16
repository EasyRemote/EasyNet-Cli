# Mission Child Target Agent Aggregate Architecture

## Root Fork

Mission execution currently reads `agents.json` and `local-agents.json` directly
while proving child targets. That duplicates Agent read-model ownership in an
execution service.

## Target Shape

- `AgentAggregateSnapshot` exposes registered Agent surface names for
  collision checks.
- `AgentAggregateSnapshot` exposes hosted Agent lookup by display name.
- Mission orchestration and Mission child Invocation gateway load the aggregate
  snapshot and consume those projection methods.
- Architecture convergence gate rejects direct Agent persistence reads in these
  Mission proof paths.

## Boundary

Agent persistence layout belongs to the aggregate owner. Mission execution owns
only EAL target semantics and child Invocation construction.
