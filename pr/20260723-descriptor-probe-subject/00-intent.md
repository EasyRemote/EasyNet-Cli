# Intent

Converge the FFI descriptor catalog probe subject selection from an ambiguous
target-owned helper into an explicit owner-kind state machine.

The user-visible failure mode is descriptor resolution for remote
`meta.list_abilities` / `invocation.history.list`: these calls must not grow
fallback subject derivation or synthetic descriptor authority. The probe may
choose a subject only from the callee owner kind that was parsed from the
canonical URA.
