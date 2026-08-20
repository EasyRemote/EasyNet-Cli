## Goal

Remove the daemon SDK handle fallback that discovers an existing daemon PID by
process-name sweep when the daemon-owned pidfile is absent.

## Non-goals

- Do not remove the CLI runtime-stop stray daemon sweep; that is an explicit
  lifecycle cleanup stage.
- Do not change daemon endpoint probing or control discovery identity checks.
- Do not change public start/attach/status API shapes.

## Acceptance criteria

- `DaemonHandle` PID projection for an attached daemon comes only from the
  relevant daemon pidfile.
- Missing or stale pidfiles produce `pid == None`, not a process-name guess.
- Endpoint readiness remains the authority for attach/start reuse.
- Focused process lifecycle tests and convergence gates pass.
