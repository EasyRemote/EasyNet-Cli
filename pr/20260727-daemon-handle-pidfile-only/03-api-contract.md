## Request contract

No new input fields are introduced.

## Response contract

`DaemonHandle::pid()` and `DaemonStatus::pid()` return:

- `Some(pid)` only when the configured pidfile contains a live PID; or
- `None` when the pidfile is missing, malformed, stale, or absent for an
  attached daemon.

## Error contract

No new errors are needed. Unknown PID is represented as `None`, while endpoint
and identity failures keep their existing typed errors.
