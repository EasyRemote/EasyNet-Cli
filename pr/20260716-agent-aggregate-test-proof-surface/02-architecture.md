# Architecture

`AgentAggregateRepository` remains the daemon persistence owner for production
Agent read projections:

- `load_snapshot` owns paired registry and hosted identity loads.
- `load_hosted_identity_status` owns governance status reads.
- `load_registered_agent` and `load_registered_agent_workspace` own
  registry-only workspace reads.

`AgentAggregateSnapshot` remains an immutable aggregate read model for
production consumers that need combined state. Methods that are only used to
prove projection behavior in unit tests are compiled only under `#[cfg(test)]`.

`HostedAgentIdentityProjection` exposes only the fields required by production
governance authorization in production builds. Profile/name fields remain
available to tests that validate exact projection behavior.
