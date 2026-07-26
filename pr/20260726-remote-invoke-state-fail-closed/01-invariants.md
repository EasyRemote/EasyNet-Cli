# Invariants

- `Completed` remains the only successful unary remote invoke state.
- Known non-completed states remain structured invocation rejection failures.
- Unknown wire states are protocol violations, not synthetic runtime states.
- Product/UI layers must never receive `UNKNOWN_STATE_*` labels.
- The adapter must keep public behavior for valid wire states.
