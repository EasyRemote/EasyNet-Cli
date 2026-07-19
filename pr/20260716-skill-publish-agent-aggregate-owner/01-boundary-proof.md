# Boundary Proof

## Owner

`src/daemon/persistence/agent_aggregate.rs` owns registered Agent read projections.

## Boundary

`src/daemon/ability/builtins/resources/skills/publish.rs` may choose skill directory layouts and perform filesystem checks. It must not load `agents.json`, inspect `AgentRegistry.agents`, or build registered-agent error inventories locally.

## Invariants

- Registered owner lookup still uses `AgentEntry::required_root_path`.
- Missing-owner errors still include the registered agent list.
- The skill ability still validates that the resolved workspace path exists before writing or reading packages.
- Agent type remains the selector for Claude Code versus generic skill directory layout.
