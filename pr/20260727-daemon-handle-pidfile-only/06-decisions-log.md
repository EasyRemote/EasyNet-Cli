## 2026-07-27

- Decision: Retire hidden `pgrep` PID projection from SDK daemon handles while
  preserving explicit CLI runtime-stop sweeps.
- Reason: PID handle ownership should be derived from daemon-owned lifecycle
  facts, not a global process-name scan that can cross state roots.
- Scope: daemon SDK process lifecycle projection only.
- Gate: Added SPEC v2 coverage that forbids `pgrep` and process-list decoding
  in `daemon::boot::process` PID projection while allowing the separate
  lifecycle stop controller to retain its explicit cleanup stage.
