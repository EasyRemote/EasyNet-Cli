# Swift Runtime Subject Boundary Convergence

## Goal

Continue cross-language SDK convergence by removing Swift runtime-subject schema
ownership from the authority support module. Runtime-state read subject
construction, resource subject classification, and retired invocation-history
subject detection must be owned by a generic runtime subject boundary.

## Invariants

- Public Swift SDK behavior and tests remain compatible.
- Runtime-state read subjects remain user-owned resource URAs under
  `runtime-state/read`.
- Retired invocation-history subject carriers remain rejected.
- Authority validation may consume runtime subject predicates but must not own
  runtime subject path constants or parse/build logic.
- Swift remains aligned with Go, Python, and Java runtime subject ownership.

## Boundary Proof

- Runtime subject construction and classification are canonical SDK runtime
  model concerns.
- Session authority validation is an authority-binding concern and should not
  duplicate runtime resource path semantics.
- Keeping Swift aligned with the existing Go/Python/Java direction prevents
  language-specific SDK architecture drift.

## Verification

- Swift runtime core seam tests focused on subject/authority behavior.
- SPEC v2 convergence gate.
- SDK product-neutrality gate.
- Architecture convergence gate.
- Formatting and diff checks.
