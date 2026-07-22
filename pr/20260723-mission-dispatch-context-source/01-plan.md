# Mission dispatch context source convergence

## Goal

Remove the remaining "env fallback" concept from the mission dispatch context
boundary without changing public behavior. The subprocess environment remains a
real transport boundary, but it must be modeled as an explicit context source
rather than a legacy fallback path.

## Root abstraction problem

`DispatchContext::current()` owns the active mission dispatch context lookup,
but it currently expresses the subprocess environment handoff as a fallback.
That vocabulary weakens the architecture boundary: future code can treat env
lookup as a degraded compatibility path instead of the canonical
cross-process handoff for child agent processes.

The dispatch boundary gate only scans `dispatch.rs`, so the actual owner file
(`context.rs`) is not protected against reintroducing fallback semantics.

## Invariants

1. In-process mission execution reads thread-local `DispatchContext`.
2. Child agent subprocesses read the serialized environment handoff.
3. Missing context remains a hard dispatch error.
4. Forged mission ids remain rejected by the existing run-dir check.
5. No production path may describe or implement mission context recovery as a
   legacy/compat/fallback path.

## Boundary proof

The clean boundary is:

```text
Mission runtime
  -> DispatchContext::enter(...)
  -> in-process dispatch reads DispatchContextSource::ThreadLocal
  -> child process receives serialize_to_env(...)
  -> child dispatch reads DispatchContextSource::ProcessEnvironment
  -> dispatch invariant validates mission id/run dir before spawn
```

The environment handoff is not an alternate authority path. It is the only
cross-process serialization channel for the same typed context tuple.

## Implementation plan

1. Introduce an explicit `DispatchContextSource` and resolved context object
   inside `context.rs`.
2. Keep `current() -> Option<DispatchContext>` for callers, backed by the
   resolved source model.
3. Rename documentation and comments from fallback to handoff/source semantics.
4. Extend `check-dispatch-mission-context-boundary.sh` and its self-test so
   the gate covers `context.rs` as well as `dispatch.rs`.
5. Add unit coverage that proves thread-local source precedence and env source
   recovery.

## Verification plan

- `bash tools/scripts/check-dispatch-mission-context-boundary.sh`
- `bash tests/scripts/test_check_dispatch_mission_context_boundary.sh`
- targeted Rust unit test for `daemon::execution::mission::context`
- `cargo fmt --check`
- `git diff --check`
- SPEC v2 and architecture gates after implementation

