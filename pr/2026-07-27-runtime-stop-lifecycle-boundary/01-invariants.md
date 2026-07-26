# Invariants

1. Runtime shape selection remains owned by `RuntimeStopPlan`.
2. Pidfile, discovery PID, and pgrep daemon sweep transitions are owned by
   `RuntimeStopProcessController`.
3. CLI stop may render outcomes but must not call `pgrep`, `kill_and_wait`,
   `is_pid_alive`, or `is_easynet_process` directly.
4. Missing or malformed pidfiles must not fabricate a runtime projection or
   claim a successful daemon stop.
