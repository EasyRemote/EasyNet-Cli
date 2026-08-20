# Intent

## Goal

Remove the lifecycle projection API and documentation that exposes `runtime.json`
as a legacy compatibility object. The runtime lifecycle layer should name the
object as a persisted session projection and provide state access through a
domain accessor.

## Non-goals

- Do not change the public CLI status or MCP output.
- Do not change the persisted `runtime.json` wire schema.
- Do not introduce a compatibility wrapper or alias for the retired accessor.

## Acceptance criteria

- `RuntimeSessionProjection` exposes a projection-state accessor instead of
  `as_runtime_state`.
- CLI and lifecycle callers read the projection through the new domain name.
- The SPEC v2 gate rejects reintroduction of legacy projection vocabulary.
- Formatting and convergence gates pass for this slice.
