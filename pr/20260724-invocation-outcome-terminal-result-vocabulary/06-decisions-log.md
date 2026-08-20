# Decisions log

- 2026-07-24: Selected Rust invocation outcome docs as the next seam because codegraph showed the only remaining `source-compatible` production symbols were `InvocationOutcome::result` and `InvocationOutcome::into_result`.
- 2026-07-24: Preserved public result accessors to maintain API compatibility while correcting their architectural contract to canonical terminal-result projection.
- 2026-07-24: Added both legacy architecture and SPEC v2 gates so the dispatch client cannot reintroduce source-compatible DTO vocabulary for terminal results.
