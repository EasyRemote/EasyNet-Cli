# Boundary Proof

## Owner

`src/daemon/persistence/agent_aggregate.rs` owns registered-Agent workspace facts derived from the durable registry.

## Boundary

`src/daemon/resources/skills/store.rs` owns skill source fetch, directory mutation, rollback, and install records. It may not reopen `agents.json` or inspect `AgentRegistry.agents` to obtain an owner workspace.

## Invariants

- Every skill mutation resolves its owner through `AgentAggregateRepository`.
- The aggregate validates the registered root with the operation-specific label (`skill.install`, `skill.upgrade`, or `skill.remove`).
- The store preserves existing user-facing missing-owner errors for each command.
- A non-missing aggregate failure propagates without being converted to a missing-owner result.
- No lifecycle mutation transaction is covered by this read-side boundary.
