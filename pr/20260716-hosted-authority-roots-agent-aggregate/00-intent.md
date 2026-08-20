# Intent

Converge hosted Agent authority-root enumeration onto the Agent aggregate
hosted-identity projection.

The current fork is a persistence facade that opens `local-agents.json`
directly to collect hosted Agent URAs for ability authority contexts. Callers
do not see the file shape, but the facade still creates a second read owner
beside `AgentAggregateRepository`.

Public behavior stays stable: callers continue to use
`daemon::persistence::hosted_agent_authority_roots()` and receive the same
ordered hosted Agent URA list.
