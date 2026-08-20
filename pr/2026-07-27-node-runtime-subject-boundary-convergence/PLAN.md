# Node Runtime Subject Boundary Convergence

## Goal

Continue cross-language SDK convergence by removing Node runtime-subject schema
ownership from the monolithic public entry module. Runtime-state read subject
construction, resource subject classification, retired invocation-history
subject detection, and all-zero principal classification must live in generic
runtime helper modules consumed by the public entry.

## Invariants

- Public Node SDK exports and TypeScript declarations remain compatible.
- Runtime-state read subjects remain user-owned resource URAs under
  `runtime-state/read`.
- Retired invocation-history subject carriers remain rejected.
- Public error objects remain SDKError instances through injected error
  factories from the public entry module.
- Node remains aligned with Go, Python, Java, and Swift runtime subject
  ownership.

## Boundary Proof

- Runtime subject construction and classification are canonical SDK runtime
  model concerns.
- Authority/session validation may consume subject predicates but must not own
  runtime resource path semantics.
- Principal all-zero classification is a generic runtime identity guard, not an
  authority-only helper.

## Verification

- Node runtime core tests.
- Node TypeScript surface tests.
- SPEC v2 convergence gate.
- SDK product-neutrality gate.
- Architecture convergence gate.
- Formatting and diff checks.
