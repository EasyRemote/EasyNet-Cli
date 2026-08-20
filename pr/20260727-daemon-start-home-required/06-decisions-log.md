## 2026-07-27

- Decision: Daemon process launch state must fail closed when a runtime home
  cannot be resolved.
- Reason: Falling back to the caller working directory creates an implicit
  product directory model and can publish sockets/logs/pidfiles under arbitrary
  workspace paths.
- Scope: daemon process lifecycle path materialization only.
- Gate: Added SPEC v2 guard coverage for daemon start HOME resolution so
  `launch_paths` and state-root helpers cannot return infallible CWD-derived
  paths again.
