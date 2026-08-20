# API Contract

## Public Surface

No public CLI, daemon RPC, stream frame, or manifest schema changes.

## Internal Contract

`AgentAggregateSnapshot` provides registered Agent projections that can be cloned into existing provider shapes where those provider signatures are already part of the module boundary.

## Error Contract

- Hot-added discover provider returns `load discover Agent aggregate` context when aggregate loading fails.
- Hot-added invoke handler returns `load invoke Agent aggregate` context when aggregate loading fails.
- Peer-skill enumeration treats aggregate load failure as an unavailable advisory context and returns no peer hints.

## Tenant Rules

This slice does not change tenant parsing or admission. Registered Agent names and entries are projected exactly as persisted by the aggregate snapshot.
