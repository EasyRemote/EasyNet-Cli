# RemoteApp Target Provider Deadline

## Intent

Bound the native host-target snapshot failure domain used by RemoteApp target
tracking. A platform API that never returns must not block daemon shutdown,
silently leave input enabled, or allow a late result from an obsolete monitor
generation to mutate a session.

This increment addresses periodic target observation only. It does not claim
that synchronous per-input focus validation is deadline-bounded, nor does it
claim cross-platform capture, network, frontend, or cross-device completion.

## Invariants

1. The RemoteDesktop plugin owns this lifecycle; no Axon Invocation or receipt
   semantics change.
2. At most one native target snapshot call may be in flight for the plugin.
3. A generation waits for a snapshot only until a monotonic deadline.
4. Deadline expiry is a generation failure and participates in the existing
   bounded restart budget and fail-safe `MonitorUnavailable` transition.
5. A late snapshot from generation N is never committed by generation N+1.
6. A permanently blocked native call cannot cause unbounded replacement
   threads or unbounded daemon shutdown.
7. Session mutation still enters only through `RemoteDesktopSessionStore`.

## Architecture

- Add one plugin-owned target snapshot executor shared by all monitor
  generations.
- The executor owns the sole in-flight native call and tags it with the monitor
  generation that started it.
- A generation waits through a bounded channel receive. On timeout it exits;
  the supervisor applies its existing failure budget.
- A replacement generation may observe completion of the prior native call,
  but discards that result by generation tag before starting a fresh snapshot.
- If the prior call remains blocked, replacements wait on the same bounded
  in-flight call rather than spawning more native threads.
- Dropping the executor detaches an irrecoverably blocked native thread; no
  shutdown path joins it indefinitely.

## Execution Checklist

- [x] Introduce the single-flight deadline executor.
- [x] Thread generation identity through platform sampling and commit.
- [x] Prove timeout, stale-result fencing, and permanent-hang boundedness.
- [x] Prove supervisor failure budget marks sessions unavailable after hangs.
- [x] Run RemoteDesktop focused/full regression and boundary gates.

## Verification

- `cargo test -p easynet target_monitor --lib`: 8 passed.
- `cargo test -p easynet daemon::plugins::remote_desktop --lib`: 385 passed.
- `cargo check -p easynet --lib`: passed.
- `check-remoteapp-lifecycle-input-boundary.sh`: passed.
- `check-remoteapp-product-closure-audit.sh`: passed.
- `remoteapp-crash-restart-recovery-e2e.sh --self-test`: passed.
- `remoteapp-product-completion-e2e.sh --self-test`: passed.

These are implementation and gate results for this increment. The two E2E
commands above ran in self-test mode and therefore are not evidence of a live
cross-device RemoteApp product run.

## Decisions

- Do not start parallel replacement native calls after a timeout; Rust cannot
  safely kill a blocked FFI thread, and parallel replacement would leak one
  thread per retry.
- Logical cancellation means the timed-out generation loses commit authority.
  It does not pretend to preempt an operating-system call that has no native
  cancellation API.
- A process boundary remains the stronger future isolation option if a target
  provider is empirically capable of permanent kernel/driver hangs.
