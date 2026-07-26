# Python Receipt Alias Guard Convergence

## Goal

Remove duplicated Python SDK production logic for rejecting the retired top-level
`receipt` alias across unary, stream, and bidi projections. The canonical SDK
wire boundary should own this as one shared runtime projection guard instead of
three facade-local compatibility checks.

## Invariants

- Public projection behavior remains fail-closed: `receipt` is rejected and
  `terminal_receipt` remains the only accepted terminal receipt field.
- Runtime, stream, and bidi projections keep their existing SDK error stage:
  `runtime`, `stream`, and `bidi`.
- No facade may reintroduce a private retired receipt alias helper.
- Go behavior is already centralized and remains unchanged.

## Boundary Proof

- The retired alias check is a runtime wire-schema invariant, not a stream,
  bidi, or unary lifecycle policy decision.
- A shared Python helper keeps the canonical wire rule cohesive while preserving
  facade-specific error staging.
- The architecture gate is updated so it requires the shared helper and rejects
  duplicated facade-local implementations.

## Verification

- Focused Python runtime/stream/bidi tests.
- SDK product-neutrality gate.
- Architecture convergence gate.
- Canonical runtime convergence v2 gate.
- `cargo fmt --check`.
