## Internal API contract

`StepDispatcher` exposes:

- `dispatch(...) -> StepDispatchOutcome`
- `dispatch_concurrency() -> StepDispatchConcurrency`
- `clone_for_thread() -> Box<dyn StepDispatcher + Send>`

## Error contract

- Sequential policy never calls `clone_for_thread`.
- Parallel policy calls `clone_for_thread`; any failure becomes a step error.
- No English-string parsing or error-as-capability inference is allowed.

## Tenant rules

No tenant derivation changes. `RunContext.tenant` continues to be passed unchanged to step dispatch.
