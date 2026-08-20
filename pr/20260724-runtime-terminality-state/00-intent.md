# Intent

## Goal

Continue runtime convergence by removing product-visible lifecycle compatibility
states where an invalid or unknown runtime handle/session can be interpreted as
a successful terminal operation.

## Non-goals

- Do not add a product-specific retry or cleanup path.
- Do not change public ABI integer values unless the canonical runtime state
  requires it.
- Do not weaken idempotency for handles that are still owned by the active
  RuntimeHandle.

## Acceptance criteria

- Identify a concrete lifecycle seam from current source evidence.
- Refactor at the state-machine owner instead of patching adapter callers.
- Add focused regression coverage plus static/SPEC coverage for the invariant.
- Verify focused tests, formatting, diff hygiene, and SPEC v2.
