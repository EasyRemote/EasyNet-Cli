# API Contract

## Public Surface

No public CLI, daemon ability, JSON, or Invocation API changes.

## Internal Contract

- `AgentAggregateSnapshot::hosted_agent_identity_by_name(name)` returns a unique hosted identity projection, missing, or a typed name lookup error.
- `AgentAggregateSnapshot::hosted_agent_identity_by_ura(agent_ura)` validates that a supplied Agent URA belongs to a local hosted identity row.
- Governance teach surfaces map aggregate results into existing teach/acquire/forget error wording.

## Failure Contract

Ambiguous hosted names, invalid stored URAs, and non-Agent stored URAs fail before authority or mutation logic runs.
