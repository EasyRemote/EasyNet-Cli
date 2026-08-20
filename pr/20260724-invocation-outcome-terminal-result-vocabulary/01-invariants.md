# Invariants

- `InvocationOutcome` owns the immutable unary invocation outcome aggregate.
- `InvocationResult` is the canonical terminal-result projection.
- `InvocationReceiptStages` owns admission and terminal checkpoint projection.
- Public API names remain stable, but docs must not imply a legacy compatibility model.
- Receipt-free outcomes remain limited to typed pre-admission failure.
