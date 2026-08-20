# Boundary Proof

Root fork:

- `src/daemon/persistence/mod.rs::hosted_agent_authority_roots()` called
  `local_agents::load()` directly and iterated `hosted_agents`.

Owner:

- `src/daemon/persistence/agent_aggregate.rs` owns hosted-Agent identity read
  projections.

Accepted path:

- `AgentHostedIdentitySnapshot` exposes hosted Agent authority roots.
- The public persistence facade delegates to
  `AgentAggregateRepository::load_hosted_identity_snapshot()`.

Rejected paths:

- The public persistence facade must not call `local_agents::load()`.
- The public persistence facade must not inspect `LocalAgentsFile` or
  `.hosted_agents`.

Effect:

- Architecture convergence: ability authority context bootstrapping consumes
  hosted identity through the same aggregate read owner as governance,
  descriptor publication, local invocation, and discovery surfaces.
- Product consistency: daemon boot and ability assembly keep their existing
  public helper and error behavior.
