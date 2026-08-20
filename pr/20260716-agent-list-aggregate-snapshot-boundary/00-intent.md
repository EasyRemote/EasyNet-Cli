# Agent List Aggregate Snapshot Boundary Intent

## Goal

Move the public `agent.list` read surface from ad-hoc paired persistence reads to one Agent aggregate snapshot owner.

## Expected Effect

- Effect convergence: `agent.list` reads registry rows and hosted-agent identity rows from one named snapshot boundary.
- Architecture cleanliness: the read-side aggregate boundary becomes explicit before broader AgentRegistry read migration.
- Product acceleration: future readers can migrate to the same snapshot API without re-learning which persistence files must be read together.

## Non-goals

- Do not change the `agent.list` request or response schema.
- Do not migrate every AgentRegistry reader in this slice.
- Do not introduce read fallbacks or compatibility layers.
- Do not change lifecycle mutation semantics.

## Acceptance Criteria

- A persistence-layer `AgentAggregateRepository` owns the paired registry/local-agent snapshot load.
- Production `agent.list` dispatch obtains its data through the aggregate snapshot owner.
- Unit fixtures can still inject deterministic snapshots without reading disk.
- The architecture convergence gate rejects regression to direct `local_agents::load()` inside `agent.list`.
