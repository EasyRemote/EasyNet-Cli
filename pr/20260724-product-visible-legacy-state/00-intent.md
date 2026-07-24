# Intent

## Goal

Remove the next product-visible legacy/compat state that makes canonical runtime
callers observe implementation-specific lifecycle, route, descriptor, or
identity behavior.

## Non-goals

- Do not add product-specific SDK behavior.
- Do not restore retired abilities or compatibility aliases.
- Do not weaken signer custody, admission, or descriptor-bound invocation.
- Do not treat a green product smoke test as proof of architecture convergence.

## Acceptance criteria

- Identify the seam with current source evidence.
- Refactor the owning abstraction instead of adding an adapter-side patch.
- Add deterministic regression coverage and, where architectural, SPEC/static
  coverage.
- Verify focused tests, formatting, diff hygiene, and the SPEC v2 gate.
