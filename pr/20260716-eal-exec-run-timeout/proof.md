# EAL Exec Run Timeout Convergence

## Root Fork

`src/daemon/execution/mission/executors/eal.rs` accepted the manifest executor
timeout but discarded it:

```rust
let _ = timeout.unwrap_or_else(|| Duration::from_secs(DEFAULT_TIMEOUT_SECS));
```

Shell and HTTP executors enforce the same manifest timeout as a concrete
execution boundary. EAL exec instead delegated to `run_mission_inproc` with no
run-level deadline, leaving only per-step EAL syntax timeouts. A manifest-bound
EAL ability therefore exposed `timeout_seconds` in the public ability binding
without enforcing the declared per-invocation SLA.

## CodeGraph Evidence

- `run_eal_exec_with_invocation_context(spec, args, invocation_context, timeout)`
  computes no effective deadline and passes no timeout into
  `run_mission_inproc`.
- `run_mission_inproc(source, MissionRunOpts)` owns the canonical mission
  execution entry point and currently calls
  `execute_with_endpoint_for_trace(...)` without a run timeout option.
- The EAL interpreter forwards only step-local `timeout N` values to
  `StepDispatcher::dispatch`; it has no run-level remaining-deadline source.
- Shell and HTTP executors enforce `timeout` directly, so EAL was the duplicate
  exception among manifest executors.

## Invariant

For a manifest-bound EAL ability, the effective executor timeout is a
run-level deadline. Every child dispatch must receive the smaller of:

- the EAL step-local timeout, when present;
- the remaining manifest executor deadline.

If the run-level deadline is exhausted before dispatch, execution must fail
closed with a timeout error instead of starting another child invocation.

## Design

- Add a narrow `run_timeout` field to `MissionRunOpts`.
- Convert `run_timeout` into a deadline at the canonical mission entry point.
- Carry that deadline through `RunContext`.
- In `execute_step_with_retry`, compute the dispatch timeout from the explicit
  step timeout and remaining run deadline.
- Make the EAL executor pass its effective manifest timeout into
  `MissionRunOpts`.

This keeps the EAL compiler/planner unchanged and avoids adding a second mission
execution path.

## Verification Plan

- Focused EAL executor unit test proving the manifest timeout reaches child
  dispatch as a bounded deadline.
- Existing EAL executor context/template tests.
- Architecture convergence script.
- Scoped formatting and whitespace checks.

## Verification Results

- `cargo test --locked --lib mission_opts_carry_manifest_timeout_as_run_deadline -- --nocapture`
- `cargo test --locked --lib run_deadline_bounds_step_dispatch_timeout -- --nocapture`
- `cargo test --locked --lib run_eal_exec_errors_on_missing_template_arg -- --nocapture`
- `cargo test --locked --lib invocation_context_is_available_to_plugin_eal_templates -- --nocapture`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tests/scripts/test_check_architecture_convergence.sh`
- `rustfmt --check --edition 2021 src/daemon/execution/mission/executors/eal.rs src/daemon/execution/mission/orchestration.rs src/eal/interpreter/mod.rs src/eal/interpreter/retry.rs src/eal/interpreter/tests.rs src/daemon/ability/builtins/automation/mission.rs src/cli/commands/groups/mission.rs src/cli/commands/agent/send.rs tests/seven_axes_w2_watch_e2e.rs`
- `git diff --check -- src/daemon/execution/mission/executors/eal.rs src/daemon/execution/mission/orchestration.rs src/eal/interpreter/mod.rs src/eal/interpreter/retry.rs src/eal/interpreter/tests.rs src/daemon/ability/builtins/automation/mission.rs src/cli/commands/groups/mission.rs src/cli/commands/agent/send.rs tests/seven_axes_w2_watch_e2e.rs tools/scripts/check-architecture-convergence.sh tests/scripts/test_check_architecture_convergence.sh pr/20260716-eal-exec-run-timeout/proof.md`
