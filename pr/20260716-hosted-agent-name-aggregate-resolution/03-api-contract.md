# API Contract

## Public Surface

No CLI argument, JSON control, or Invocation API changes.

## Internal Contract

`AgentAggregateSnapshot::hosted_agent_ura_by_name` returns:

- `Ok(Some(&str))` for one canonical hosted Agent URA.
- `Ok(None)` when no hosted Agent has that display name.
- `Err(HostedAgentNameLookupError)` for ambiguous names, invalid URAs, or non-Agent URAs.

Consumers map that result into their existing surface-specific errors.

## Error Contract

Aggregate load failures preserve source-classified context. Missing and ambiguous hosted names remain hard errors on explicit Agent-name requests.

## Tenant Rules

Resolved values must be canonical Agent URAs. A non-Agent URA in the hosted identity projection is rejected before dispatch/delegation.
