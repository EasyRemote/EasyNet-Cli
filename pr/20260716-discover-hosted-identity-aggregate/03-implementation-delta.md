Implementation Delta
====================

Domain changes:
- Added AgentHostedIdentitySnapshot as a hosted-identity-only aggregate
  projection.
- Added AgentAggregateRepository::load_hosted_identity_snapshot().
- Reused one hosted LLM lookup implementation for full AgentAggregateSnapshot
  and hosted-only AgentHostedIdentitySnapshot.

Caller migration:
- CLI ability catalogue `--agent` resolution now uses the hosted identity
  snapshot instead of local_agents::load().
- agents.discover local owner projection now uses the hosted identity snapshot
  instead of local_agents::load() and lookup_hosted_ura().

Gate:
- Added R42_HOSTED_OWNER_LOOKUP_AGENT_AGGREGATE_FORK to prevent hosted owner
  lookup surfaces from reviving direct local-agents reads or full aggregate
  snapshot reads.

Compatibility:
- Updated the remote system ability error context from "invoke" to "forward" so
  the existing remote ability-list test contract continues to expose the forward
  path in grep-friendly output.
