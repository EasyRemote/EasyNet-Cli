# Agent List Aggregate Snapshot API Contract

## Public Request Contract

No public API change.

- Request remains an empty object.
- Additional properties remain rejected by descriptor schema.

## Public Response Contract

No public response change.

- Response remains `{ "agents": [...] }`.
- Row fields and nullability are unchanged.

## Authority Contract

- `agent.list` remains Device-owned.
- The aggregate snapshot does not change admission, owner routing, or URA construction.
