## Boundary

The EAL interpreter is daemon-owned orchestration. It may choose sequential or parallel execution for a phase, but that choice is runtime policy, not protocol semantics.

## Refactoring direction

- Add a small internal dispatch concurrency policy enum at the `StepDispatcher` boundary.
- Make each dispatcher implementation declare its policy.
- Keep `clone_for_thread` as the parallel worker factory only.
- Remove caller-side capability probing via `clone_for_thread().is_ok()`.

## Layering

- EAL interpreter owns phase scheduling.
- `StepDispatcher` implementations own their thread-safety declaration.
- Mission invocation gateway remains the only child Invocation submitter.
