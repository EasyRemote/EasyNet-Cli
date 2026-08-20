# Java Runtime Subject Boundary Convergence

## Goal

Continue cross-language SDK convergence by removing Java runtime-subject schema
ownership from the authority support helper. Runtime-state read subject
construction and retired invocation-history subject detection must be owned by
the generic runtime subject boundary, matching the Go/Python direction.

## Invariants

- Public Java SDK behavior and tests remain compatible.
- Runtime-state read subjects remain user-owned resource URAs under
  `runtime-state/read`.
- Retired invocation-history subject carriers remain rejected.
- Authority support may consume runtime subject predicates but must not own
  runtime subject path constants or parse/build logic.

## Boundary Proof

- Runtime subject construction and classification are canonical SDK runtime
  model concerns.
- Session authority validation is an authority-binding concern and should not
  duplicate resource path knowledge.
- Keeping Java aligned with Go/Python prevents language-specific runtime subject
  drift.

## Verification

- Java runtime core seam tests focused on subject/authority behavior.
- SPEC v2 convergence gate.
- SDK product-neutrality gate.
- Architecture convergence gate.
- Formatting and diff checks.
