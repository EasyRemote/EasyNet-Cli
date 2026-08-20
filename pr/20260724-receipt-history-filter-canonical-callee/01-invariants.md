# Invariants

## Semantic invariants

- Receipt history is a canonical invocation-ledger query surface, not a product directory query surface.
- Invocation tuple predicates are named by tuple role: caller, callee, subject, ability, state, trace.
- A callee can be a Device, Agent, or Authority URA; the field name must remain `callee_ura` regardless of product role.

## Safety invariants

- The daemon receipt-history parser must reject unknown or retired filter fields instead of remapping them.
- SDK filters must not expose product-specific aliases that can diverge across languages.
- CLI facade compatibility must happen before the daemon wire shape and must not create a second accepted runtime protocol.

## Boundedness invariants

- Rejection happens synchronously during request parsing before ledger reads.
- Query construction has a single callee predicate path.
