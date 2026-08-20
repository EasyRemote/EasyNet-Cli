# Intent

Converge `agent.list` row projection onto the Agent aggregate read model.

`agent.list` already loads an `AgentAggregateSnapshot`, but its row builder
accepts raw `AgentRegistry` plus `LocalAgentsFile` and calls
`lookup_hosted_ura()` directly. That keeps hosted identity lookup logic inside
the ability view after the source-of-truth was moved to the aggregate.

Public behavior stays stable: the ability still lists registered agents with
runtime, model, label, timeout, root metadata, and a hosted LLM owner URA when
one exists.
