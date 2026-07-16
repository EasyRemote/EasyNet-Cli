# Architecture

## Boundary

`src/daemon/persistence/agent_aggregate.rs` owns paired Agent read state. `src/daemon/ability/health.rs` is a monitor consumer that converts aggregate facts plus manifests into health scan work.

## Layering

- Persistence/domain: `AgentAggregateRepository` loads durable registry and hosted identity projection exactly once per scan.
- Persistence/domain: `AgentAggregateSnapshot` owns hosted LLM identity selection.
- Ability health: `scan` maps aggregate facts to monitored/unmonitored ability URAs and keeps record scheduling local.

## Expected Effect

This removes another public daemon read-side fork. A health scan can no longer observe registry rows and hosted LLM URAs from different independent loads, and corrupt duplicate hosted identity rows no longer select an arbitrary owner for health metadata.
