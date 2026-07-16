# Boundary Proof

## Owners

- `AgentAggregateRepository::load_snapshot` owns paired registry and hosted-identity reads.
- `AgentAggregateRepository::load_registered_agent` owns registry-only registered-Agent projections.

## Invariants

- Registry-only callers read `agents.json` once and never read `local-agents.json`.
- Package mutations receive a validated root path and preserve command-specific missing-owner errors.
- Ability authoring receives the same validated workspace plus the immutable registered runtime entry needed for live catalog synchronization and rollback.
- Paired snapshot callers retain source-classified registry versus identity failures.
- No compatibility fallback treats an unreadable registry as an empty registry.
