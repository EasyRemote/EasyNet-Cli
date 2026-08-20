# Boundary Proof

## Ownership

The language facade owns typed request/projection objects, request validation,
stream handle adaptation, and lifecycle error surfacing. The daemon provider
owns live directory state, event production, resume semantics across process
boundaries, and product fan-out policy.

## Invariants

- Subscription requests carry complete caller, callee, subject, nonce, causal
  context, and descriptor-version fields.
- Directory subscription uses `resume_cursor`; it does not reuse list
  pagination state.
- Subscription carriers target `directory.subscribe`.
- Projection state is bounded to `1024` buffered events.
- Snapshot and live events are projected as SDK DTOs, not raw daemon internals.
- Missing subscription transport fails explicitly.

## Compatibility

The change adds Directory subscription APIs without changing existing list,
resolve, or identity projection behavior.
