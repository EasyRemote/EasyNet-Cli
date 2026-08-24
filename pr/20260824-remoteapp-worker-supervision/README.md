# RemoteApp Worker Supervision

## Intent

Make the device-owned RemoteDesktop plugin keep target tracking alive when its
poll worker faults during an active session. Recovery happens inside the
plugin lifecycle boundary and rebuilds a new poll generation from
supervisor-owned desired session state without requiring another public
Invocation.

Close the durability race that otherwise lets a delayed active snapshot
overwrite a later terminal snapshot and revive a closed interactive session
after daemon restart.

This change does not claim hung host-provider recovery, daemon-process crash
recovery, WebRTC media reattachment, terminal receipt replay, or cross-device
readiness. Those remain separate live evidence rows. A blocked native provider
requires a cancellable/deadline-aware provider boundary plus generation
fencing; Rust threads cannot be safely killed and must not be replaced in
parallel without fencing stale commits.

## Invariants

1. The SystemAgent-owned `remote_desktop.*` AbilityDescriptors and public
   Invocation tuple do not change.
2. The RemoteDesktopPlugin remains the sole owner of monitor lifecycle and
   session mutation; no test script mutates session state.
3. A poll-generation panic or unexpected exit cannot silently terminate
   tracking for active sessions. Native-provider stalls are explicitly outside
   this increment.
4. Recovery preserves the same session id and selected Resource binding.
5. Recovery attempts use bounded, interruptible backoff and do not spin.
6. Terminal recovery state is absorbing; delayed active writes cannot revive a
   closed session.
7. Target mutations performed by the monitor request a durable aggregate
   snapshot, without fsyncing unchanged poll ticks.
8. Repeated generation failure exhausts a budget and marks target health
   unavailable, disabling unsafe input/media state instead of silently
   remaining healthy.

## Architecture

- `target_monitor.rs` owns a stable supervisor thread plus a replaceable poll
  generation. The supervisor owns desired session ids, generation numbering,
  backoff, and the failure budget.
- `lifecycle_worker.rs` allows an owner to replace a previously panicked worker
  rather than treating the old panic as a permanent restart failure.
- `session_recovery.rs` takes a process-spanning per-session lock, rejects
  stale/active-after-terminal commits within the same incarnation, fails
  closed when an existing row cannot be read, and reuses the shared
  unique-temp, fsync, atomic-rename writer. A newer session token/creation time
  identifies a new incarnation; delayed rows from the older incarnation are
  rejected.
- `session.rs` / `session_store.rs` remain the only aggregate mutation path.
- The first debounced target miss emits `TARGET_LOSS_PENDING`, so immediate
  input deactivation is durable without falsely emitting `TARGET_LOST`.
- Restart-budget exhaustion commits the distinct non-debounced
  `MonitorUnavailable` observation, stops media by transport epoch, and
  persists the target-tracker projection for restart rehydration.
- A test-only fault trigger exercises a real poll-thread panic; release builds
  expose no public crash API.
- `remoteapp-crash-restart-recovery-e2e.sh` remains an evidence verifier; a
  host runner must drive the real daemon/plugin paths and feed its artifact.

## Execution checklist

- [x] Add a bounded target-monitor supervision state machine.
- [x] Make terminal recovery snapshots absorbing and race-safe.
- [x] Persist changed monitor observations without persisting healthy no-op
      ticks.
- [x] Add deterministic unit coverage for fault, backoff, recovery, and
      preserved desired tracking.
- [x] Fail closed for unreadable/corrupt existing recovery rows and persist
      target-tracker safety state.
- [x] Make restart-budget exhaustion a confirmed target-unavailable transition
      rather than one ordinary debounced target miss.
- [ ] Add a host fault-injection runner without adding a public product API.
- [ ] Add a cancellable host-provider deadline and generation fencing for
      native poll hangs.
- [x] Update closure gates and readiness documentation without claiming full
      crash/restart completion.
- [x] Run focused Rust, script, mutation, and diff checks.

## Verification

- `cargo test -p easynet lifecycle_worker --lib` — PASS (2 tests).
- `cargo test -p easynet target_monitor --lib` — PASS (6 tests), including
  three real generation panics followed by durable target-unavailable state.
- `cargo test -p easynet session_recovery --lib` — PASS (12 tests).
- `cargo test -p easynet target_observer --lib` — PASS (23 tests).
- `cargo test -p easynet daemon::plugins::remote_desktop --lib` — PASS
  (383 tests).
- `cargo check -p easynet --lib` — PASS.
- Crash/product verifier self-tests — PASS.
- Lifecycle boundary and product closure source gates — PASS.
- `git diff --check` — PASS.
- Earlier rejected draft: `cargo test -p easynet target_monitor --lib` — FAIL
  (3 pass, 2 fail). That implementation was removed; it caught panic in the
  same thread and had an unsafe snapshot race.

## Decisions

- Do not call a verifier-only script product evidence.
- Do not model an in-process target-monitor thread as an independently
  deployed plugin process.
- Do not reuse Axon Invocation terminal semantics as a worker restart signal.
- Do not publish worker lifecycle events until their durable ordering and
  binding-identity comparison are explicit. This change proves generation
  restart internally and uses existing target-unavailable state after the
  failure budget.
- Daemon restart, target-monitor restart, and media-worker restart are three
  different scenarios. A daemon/media restart must mint a new transport epoch;
  preserving the public session does not permit stale transport callbacks.
