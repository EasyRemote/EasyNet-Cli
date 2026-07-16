# Boundary Proof

Root fork:

- `src/daemon/ability/catalog/profiles/mod.rs` loaded `local-agents.json`
  directly, looked up consent/MCP owners, and iterated hosted LLM rows.

Owner:

- `src/daemon/persistence/agent_aggregate.rs` owns hosted identity read
  projections.

Accepted path:

- `AgentHostedIdentitySnapshot::host_descriptor_identity_projection()` owns the
  descriptor identity shape.
- The descriptor catalog consumes `AgentHostDescriptorIdentityProjection` and
  only validates descriptor owner URAs.

Rejected paths:

- The descriptor catalog must not call `local_agents::load()`.
- The descriptor catalog must not call `lookup_hosted_ura()`.
- The descriptor catalog must not inspect `LocalAgentsFile`, `.hosted_agents`,
  or `host_device_agent_ura` storage fields.

Effect:

- Architecture convergence: hosted identity file shape is hidden behind one
  aggregate projection.
- Product consistency: MCP stdio and in-process descriptor callers keep the
  same published catalog behavior.
