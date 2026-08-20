# Child Invocation Receipt Anchor Neutrality

## Goal

Remove Mission-named receipt/invocation DTOs from the EAL receipt graph boundary. EAL may remain a Mission executor, but the receipt anchors it passes between steps are canonical child-invocation facts, not Mission product objects.

## Invariants

1. Dependency edges between EAL steps are represented as child invocation receipt anchors.
2. The run-level receipt graph projects canonical invocation and terminal receipt facts without product-specific wrapper names.
3. Mission remains an upstream executor/orchestrator consumer; it must not own the generic receipt graph abstraction.
4. No compatibility aliases are left behind for retired MissionReceiptReference or MissionInvocationRecord names.
5. Existing public behavior stays compatible: EAL output still includes the same JSON fields for invocation and terminal receipt facts.

## Boundary Proof

- Lower layer: Axon/LocalRuntime owns descriptor-bound admission and signed terminal receipts.
- Daemon execution layer: Mission gateway stages product policy and dispatches child invocations.
- EAL interpreter: consumes only generic child-invocation receipt anchors/records for dependency propagation.

## Verification Plan

- Targeted Rust tests covering EAL receipt dependency graph.
- Canonical runtime convergence v2 gate.
- Architecture convergence gate.
- `cargo fmt --check`, `git diff --check`.
- Codegraph status/sync after edits.
