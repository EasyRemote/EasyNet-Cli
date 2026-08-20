# Invariants

- `restart_recover` is a generic runtime lifecycle action, not an EasyNet daemon or product lifecycle.
- SDK facades do not scan persistence, reap processes, or fabricate receipts.
- Recovery requests are bounded by explicit deadline and maximum invocation count.
- A provider report is accepted only when it proves `runtime_started`, `bounded_scan`, and `cleanup_complete`.
- Recovery events and replayed terminal receipt counts are observable in both Go and Python.
- Ability facades delegate recovery to Runtime Core instead of defining a parallel state machine.
