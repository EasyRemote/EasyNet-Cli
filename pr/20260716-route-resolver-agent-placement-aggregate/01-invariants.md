# Invariants

- Hosted-Agent placement is a projection of aggregate Agent identity state.
- A route can prove local hosted placement only when the aggregate projection is available and the placement host equals this daemon's device URA.
- Aggregate projection load failure must not prove local hosted placement.
- Route resolver does not own the `local-agents.json` file shape.
- Existing route selection order and public route metadata remain unchanged.
