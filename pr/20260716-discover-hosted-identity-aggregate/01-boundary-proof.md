Boundary Proof
==============

Root fork:
- agents.discover loaded local-agents.json directly to map local LLM Agent names
  to hosted owner URAs.
- CLI ability catalogue filtering loaded local-agents.json directly to resolve
  `--agent` to a hosted owner URA.

Owner:
- src/daemon/persistence/agent_aggregate.rs owns hosted-Agent identity read
  projections.

Accepted path:
- agents.discover consumes AgentHostedIdentitySnapshot.
- CLI ability catalogue filtering consumes AgentHostedIdentitySnapshot.
- AgentHostedIdentitySnapshot exposes hosted_llm_agent_ura().
- AgentAggregateRepository owns the durable hosted identity file read.

Rejected paths:
- agents.discover must not call local_agents::load().
- CLI ability catalogue filtering must not call local_agents::load().
- agents.discover must not inspect LocalAgentsFile.
- CLI ability catalogue filtering must not inspect LocalAgentsFile.
- agents.discover must not use full AgentAggregateSnapshot for owner URAs,
  because that would couple hosted identity lookup to registry readability.
- CLI ability catalogue filtering must not use full AgentAggregateSnapshot for
  owner URAs for the same reason.
