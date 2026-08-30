# API Contract

## Public behavior

- No public ability names, URA shapes, or SDK interfaces change.
- `watch_events` reconnects with `from_sequence = last_committed_sequence + 1`.
- A session remains visible in `closing` after an ambiguous end response and is
  removed only after terminal reconciliation.
- The original `session.subjectUra` remains authoritative after inventory drift.

## Private platform contract

- `ProcessInstance::resolve(pid)` returns the canonical stable identity for the
  current OS process generation.
- Linux window ownership is obtained only through XRes local-client PID.
- `CaptureEligibleSurface` is a pure predicate shared by inventory and capture.
