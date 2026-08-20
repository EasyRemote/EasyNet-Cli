# Mission Child Target Agent Aggregate API Contract

## Aggregate Owner

`AgentAggregateSnapshot` provides:

- `registered_agent_surface_names() -> BTreeSet<String>`
- `hosted_agent_by_name(name) -> Result<HostedAgentNameResolution, ...>`

The aggregate owner performs hosted identity ambiguity detection and URA shape
validation.

## Mission Consumers

Mission consumers may:

- load a snapshot through `AgentAggregateRepository`,
- check surface-name collisions,
- resolve a hosted Agent child callee URA.

They must not parse or load Agent persistence files directly.
