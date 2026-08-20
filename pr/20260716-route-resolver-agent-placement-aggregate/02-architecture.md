# Architecture

## Boundary

`AgentAggregateSnapshot::hosted_agent_placements` owns conversion from hosted identity rows to route placement facts. `LocalHostedAgentPlacements` is now a route-local adapter over that aggregate projection.

## Layering

- Persistence/domain: aggregate snapshot owns hosted placement projection and host device parsing.
- Routing: route resolver converts aggregate placement facts into its local route proof type.
- Invocation dispatch: existing resolver consumers continue to use `DaemonRouteResolver` unchanged.

## Expected Effect

This removes hosted placement file-shape knowledge from namespace route resolution. Route proofs no longer depend on a route-local parser for hosted Agent URAs or host device placement.
