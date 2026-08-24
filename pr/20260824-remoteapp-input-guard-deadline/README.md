# RemoteApp Input Guard Deadline

## Product failure

Target-local keyboard and pointer frames synchronously enumerate host windows
immediately before OS injection. If the platform API hangs, the WebRTC data
channel task currently hangs with it, preventing bounded rejection, cancel,
and session shutdown behavior.

## Invariants

1. Periodic target observation and per-frame input validation share the same
   plugin-owned native snapshot failure domain.
2. At most one host target snapshot call may be in flight per plugin.
3. Input waits use a short monotonic deadline and reject fail-closed on expiry.
4. A result created for a monitor generation or older input request cannot be
   committed by another authority.
5. Timeout does not pretend to preempt an OS API without native cancellation.
6. Input policy, target binding, focus epoch, geometry revision, and pointer
   occlusion validation remain unchanged after a successful snapshot.
7. No session-store lock is held while waiting for a host snapshot.

## Execution

- [x] Generalize the existing single-flight executor to typed request owners.
- [x] Route keyboard and pointer live-target validation through that executor.
- [x] Prove bounded timeout, stale-result fencing, and recovery after release.
- [x] Keep monitor hang tests and all RemoteDesktop regressions green.
- [x] Extend source-contract and product-closure gates.

## Scope boundary

This increment bounds live target validation. It does not claim that the OS
input injection API itself is preemptible; platform injection permission and
injection-call deadlines remain separate product evidence.

## Verification

- `cargo test -p easynet target_monitor --lib`: 9 passed.
- `target_local_input_provider_hang_rejects_with_bounded_deadline`: passed.
- isolated `cargo test -p easynet daemon::plugins::remote_desktop --lib` against
  clean `ff6c1fb70` plus this exact diff: 387 passed.
- the original checkout `cargo check -p easynet --lib` passed before the final
  cohesion-only executor module extraction; the isolated full test build then
  compiled that final extraction. A redundant isolated `cargo check` was
  stopped by `ENOSPC` after the full test build had already passed.
- lifecycle/input boundary gate: passed.
- product-closure audit gate: passed.
- crash/restart and product-completion harness self-tests: passed.

The harness commands above ran in self-test mode and are not live cross-device
product evidence.
