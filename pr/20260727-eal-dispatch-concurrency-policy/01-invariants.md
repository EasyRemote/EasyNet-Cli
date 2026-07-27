## Invariants

- One EAL step dispatch still produces exactly one canonical child Invocation record on success.
- Phase execution remains deterministic: phases are barriers; call partitions execute required steps before optional steps.
- Parallel execution is an explicit lifecycle state of the dispatcher, not an error side effect.
- `clone_for_thread` failure in a parallel-declared dispatcher is a structural runtime error, not a signal to silently degrade.
- Sequential-only dispatchers must be inspectable before dispatch starts.
