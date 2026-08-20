Intent: retire the legacy projection that collapses route owner liveness
failures into ability/descriptor absence.

Observed product symptom:
- `invocation.history.list` and `meta.list_abilities` can surface
  `ABILITY_NOT_FOUND` with embedded `ROUTE_NEGATIVE ... owner is not online`.

Architecture target:
- A route owner that is offline is an availability terminal state, not a missing
  ability.
- Rust daemon status, Go SDK, and Python SDK must converge on the same public
  code: `DESCRIPTOR_OWNER_OFFLINE`.
