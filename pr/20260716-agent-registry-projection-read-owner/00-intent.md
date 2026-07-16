# Intent

## Root fork

Several daemon read paths reopen `agents.json` directly although
`AgentAggregateRepository` owns Agent read projections. The existing aggregate
snapshot is deliberately unsuitable for these paths because it also requires
`local-agents.json`.

## Objective

Make the repository the single owner of full registry-only projections, then
migrate bootstrap planning, curator catalog collection, daemon catalog boot,
and post-purge catalog replay. No caller may acquire a registry projection by
loading hosted identity state.

## Public behavior

- Bootstrap and daemon boot preserve their existing registry error context.
- Curator catalog collection remains best effort and yields an empty catalog on
  registry failure or a missing owner.
- Hub-only catalog construction continues to avoid device registry reads.
