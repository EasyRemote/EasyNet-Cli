## Invariants

- A PID attached to a daemon handle must be traceable to the daemon-owned
  pidfile for the same state root.
- Endpoint probes prove transport liveness; they do not authorize a global
  process-name PID sweep.
- A stale pidfile is ignored for PID projection.
- Missing pidfile is an unknown PID state, not permission to infer ownership
  from another process list.
- CLI cleanup may still execute its explicit lifecycle sweep outside the SDK
  handle projection path.
