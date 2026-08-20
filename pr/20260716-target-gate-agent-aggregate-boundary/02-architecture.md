# Architecture

## Boundary

`src/daemon/persistence/agent_aggregate.rs` owns paired Agent read state. `TargetGate` is an admission consumer that derives an in-memory locality index from that snapshot.

## Layering

- Persistence/domain: `AgentAggregateRepository` loads `agents.json` and `local-agents.json` once and returns an immutable snapshot.
- Persistence/domain: `AgentAggregateSnapshot::local_target_projection` owns the hosted Agent target value object and registered Agent ID projection.
- Admission: `LocalAgentTargetIndex` consumes the aggregate-owned projection through an explicit available/unavailable state.
- Dispatch: unary, stream, and bidi dispatchers continue to consume `TargetGate` without API changes.

## Expected Effect

This slice reduces source-of-truth splitting in the invocation admission path. The concrete product effect is stable local dispatch for hosted Agent URAs: a self-target decision cannot combine hosted identities from one read with registry IDs from a later independent read, and a failed aggregate read cannot silently accept partial Agent evidence.
