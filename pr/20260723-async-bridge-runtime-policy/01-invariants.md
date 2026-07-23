# Invariants

## Semantic invariants

- Multi-thread Tokio callers still use `block_in_place` with the current handle.
- Current-thread Tokio callers still avoid illegal runtime re-entry.
- Callers that require Tokio resources still use a fresh current-thread runtime.
- In-memory futures may still use `futures::executor::block_on`.

## Safety invariants

- No implicit runtime strategy is introduced.
- No compatibility alias preserves the retired policy type.
- Runtime construction failures remain typed through `try_run_blocking`.

## Boundedness invariants

- The bridge creates at most one helper runtime/thread per call for policies
  that require Tokio resources.
- Detached workers remain owned by `spawn_current_thread_tokio`.
