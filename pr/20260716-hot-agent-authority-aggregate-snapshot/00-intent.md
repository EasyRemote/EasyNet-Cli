# Hot Agent Authority Aggregate Snapshot Intent

## Goal

Move hot-agent authority enrollment and durable-removal proof reads onto the Agent aggregate snapshot owner.

## Expected Effect

- Effect convergence: authority inventory now verifies registry presence and hosted-agent identity against one aggregate snapshot boundary.
- Architecture cleanliness: the proof path no longer assembles `agents.json` and `local-agents.json` reads inside dispatch code.
- Product acceleration: future Agent read migrations can reuse the same typed snapshot lookup methods.

## Non-goals

- Do not change public admission or invocation behavior.
- Do not change authority inventory error variants.
- Do not migrate all remaining AgentRegistry readers in this slice.
- Do not add fallback identity lookup paths.

## Acceptance Criteria

- `AgentAggregateRepository` exposes a source-preserving snapshot load path.
- `AgentAggregateSnapshot` owns registered-agent and hosted LLM identity lookup helpers.
- `PersistedHotAgentAuthority::load` uses the aggregate snapshot instead of direct persistence reads.
- `HotAgentAuthorityInventory::revoke_after_durable_removal` uses the aggregate snapshot instead of direct persistence reads.
- Architecture convergence gate rejects direct registry/local-agents reads inside these authority proof paths.
