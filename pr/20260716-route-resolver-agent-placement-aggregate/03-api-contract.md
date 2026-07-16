# API Contract

## Public Surface

No public route API, Invocation API, or catalog API changes.

## Internal Contract

`LocalHostedAgentPlacements::load` loads one Agent aggregate snapshot and converts `AgentHostedPlacementProjection` into route-local placement records.

## Error Contract

Aggregate load failure logs an operational event and leaves placement unavailable. Unavailable placement does not prove local hosted Agent routing.

## Tenant Rules

Placement matches only when the hosted Agent URA is present in the aggregate projection and its host device URA equals this daemon's local authority device URA.
