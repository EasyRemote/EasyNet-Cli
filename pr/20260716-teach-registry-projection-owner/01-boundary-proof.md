# Boundary Proof

`resolve_owner_manifest` needs a registered Agent workspace, not hosted Agent
identity. `recover_forget_transactions` needs the current registered runtime
entry after a durable descriptor-removal transaction, not hosted identity.

Both paths therefore consume
`AgentAggregateRepository::load_registered_agent_registry_projection`. They
must not use `load_snapshot`, because a malformed `local-agents.json` must not
block descriptor transfer validation or recovery of a durable forgetting row.

`TeachGrantStore` remains the lifecycle transaction owner. The recovery path
only observes the post-transaction registry projection to decide whether live
runtime convergence is required or has become terminally unnecessary.
