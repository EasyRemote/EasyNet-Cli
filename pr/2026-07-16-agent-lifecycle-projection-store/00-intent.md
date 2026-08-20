# Agent Lifecycle Projection Store

## Intent

Start converging the Agent aggregate root fork by giving the `agent.start`,
`agent.stop`, and local purge-recovery lifecycle path one private owner for the
paired durable projections:

- `agents.json`
- `local-agents.json`

## Expected effect

- **Effect type:** architecture convergence.
- **Root fork addressed:** Agent aggregate ownership split between lifecycle
  handlers and raw persistence modules.
- **Concrete use case:** a lifecycle transition should not hand-assemble
  registry and hosted-identity persistence at each call site. The transition
  state machine should delegate projection writes to one cohesive object that
  names the lifecycle boundary and can later become the enforcement point for a
  full `AgentAggregate`.

## Non-goals

- Do not change the public `agent.start`, `agent.stop`, or `agent.purge`
  response contract.
- Do not migrate unrelated fixture/test writes.
- Do not introduce compatibility fallbacks.
- Do not promote the full Agent aggregate gate until direct production callers
  have been migrated through the aggregate/application service.
