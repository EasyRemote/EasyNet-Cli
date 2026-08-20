# Invariants

1. The FFI layer observes terminal state; it does not define a second lifecycle vocabulary.
2. Terminal-state strings accepted by the FFI projection must match the canonical public ABI exactly.
3. Case-insensitive parsing is a compatibility path because it admits non-canonical lifecycle states.
4. Receipt-chain validation and terminal monotonicity remain owned by `InvocationOutcome` and handle lifecycle code.
5. Public behavior remains compatible for canonical clients that already consume `Completed`, `Failed`, `TimedOut`, or `Cancelled`.
