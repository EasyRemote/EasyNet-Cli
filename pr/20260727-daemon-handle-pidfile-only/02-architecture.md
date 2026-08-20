## Boundary

`DaemonHandle` is a lifecycle handle, not a process discovery scanner. It may
carry a PID only when that PID was written by the daemon lifecycle authority for
the matching runtime state directory.

## Refactoring direction

- Remove `pgrep -f easynet-daemon` from `daemon::boot::process`.
- Keep `read_daemon_pid_at` and liveness filtering as the sole PID projection.
- Add regression tests proving missing and stale pidfiles do not synthesize a
  PID.
- Extend SPEC v2 gate coverage to keep the hidden SDK handle sweep retired.

## Ownership

- `daemon::boot::process` owns SDK handle PID projection.
- `daemon::boot::lifecycle::stop` owns explicit CLI cleanup sweeps.
- Endpoint readiness remains separate from PID projection.
