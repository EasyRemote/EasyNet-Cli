# Intent

## Goal

Remove active legacy or compatibility seams from product-visible runtime ingress paths so CLI/UI calls converge on the canonical runtime model instead of carrying stale authority, route, descriptor, or product-specific state.

## Non-goals

- Do not add compatibility fallbacks for stale local data.
- Do not weaken daemon admission or descriptor validation to make current product flows pass.
- Do not introduce EasyNet/EasyRemote-specific SDK abstractions.

## Acceptance criteria

- One identified active legacy seam is removed at its root abstraction.
- Callers migrate to the canonical owner of the behavior.
- Obsolete code and tests for the removed seam are deleted or rewritten.
- Targeted tests and architecture gates prove the new boundary.
