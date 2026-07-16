# Boundary Proof

Root fork:

- `src/daemon/identity/local_invocation.rs` reads `local_agents::load()` for
  `host_device_agent_ura`.
- `src/daemon/resources/context/clipboard_tracker.rs` reads
  `local_agents::load()` for the same field.

Owner:

- `src/daemon/persistence/agent_aggregate.rs` owns hosted-Agent identity read
  projections, including the host device URA status already consumed by
  governance status and network health.

Accepted path:

- Host device URA consumers call
  `AgentAggregateRepository::load_hosted_identity_status()`.
- Callers use `AgentHostedIdentityStatus::host_device_agent_ura()` rather than
  inspecting `LocalAgentsFile`.

Rejected paths:

- daemon local invocation identity must not call `local_agents::load()`.
- clipboard tracking must not call `local_agents::load()`.
- either surface must not inspect `LocalAgentsFile` or the
  `host_device_agent_ura` storage field directly.

Effect:

- Architecture convergence: hosted identity read ownership becomes aggregate
  owned across local invocation and context-resource surfaces.
- Product consistency: no observable command or daemon behavior changes.
