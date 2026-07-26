# SDK Runtime Subject Boundary Convergence

## Goal

Continue removing legacy/compat logic from canonical SDK runtime paths by
converging runtime subject predicates into one generic subject boundary per
language. The immediate target is the runtime-state read subject and retired
invocation-history subject checks currently spread across session-authority and
authority helpers.

## Invariants

- Public SDK API names and error behavior remain compatible.
- Canonical runtime-state read subjects remain user-owned resource URAs under
  `runtime-state/read`.
- Retired invocation-history carrier subjects remain rejected, but the rejection
  must be owned by a runtime subject boundary rather than duplicated inside
  authority-specific logic.
- Go and Python SDKs converge structurally; neither should gain a language-only
  subject model.

## Boundary Proof

- A subject predicate is a runtime identity/schema concern, not authority
  transport policy.
- Authority code may ask whether a subject is canonical or retired, but it must
  not own path parsing logic for specific historical resource carriers.
- Centralizing subject parsing reduces legacy surface area and keeps future
  session subject tightening localized.

## Verification

- Go authority/authorized-runtime-session/runtime-ability focused tests.
- Python authority/authorized-runtime-session/runtime-ability focused tests.
- SDK product-neutrality gate.
- Architecture convergence gate.
- Canonical runtime convergence v2 gate.
- `cargo fmt --check`.
