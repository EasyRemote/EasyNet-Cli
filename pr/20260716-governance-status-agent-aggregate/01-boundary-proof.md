Boundary Proof
==============

Root fork:
- Governance read handlers know the local-agents persistence shape.

Owner:
- src/daemon/persistence/agent_aggregate.rs owns paired Agent read models and
  hosted-Agent identity projections.

Accepted read model:
- AgentHostedIdentityStatus exposes joined state, optional host-device Agent
  URA, and hosted-Agent count.

Rejected paths:
- Governance handlers must not call local_agents::load().
- Governance handlers must not inspect LocalAgentsFile.host_device_agent_ura or
  LocalAgentsFile.hosted_agents directly.
- No compatibility fallback is introduced; the file-shape knowledge is moved
  to the aggregate repository.
