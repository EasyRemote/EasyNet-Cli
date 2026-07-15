# Invariants

1. `cli start` must not call `local_agents::save` directly.
2. Startup hosted identity projection must be an Agent lifecycle operation.
3. The operation must acquire `AgentLifecycleMutationGuard` before writing
   `local-agents.json`.
4. Identity persistence must flow through `AgentLifecycleProjectionStore`.
5. The bootstrap plan and `bootstrap_local_agents` projection semantics remain
   unchanged.
6. This slice does not claim to remove registry load-time migration writes.
