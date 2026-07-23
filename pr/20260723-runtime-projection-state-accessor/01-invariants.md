# Invariants

## Semantic invariants

- `runtime.json` remains metadata about the operator/session projection, not
  process authority.
- Process truth remains owned by daemon discovery and lifecycle status
  classification.
- Projection reads must not mutate persisted state.

## Safety invariants

- No caller may infer daemon liveness only from the projection.
- The persisted schema remains unchanged until an explicit public schema
  migration is designed.
- The retired accessor must not remain as a compatibility alias.

## Boundedness invariants

- The change is synchronous, read-only, and introduces no new runtime path.
- The convergence gate checks only source vocabulary and required accessor
  shape.
