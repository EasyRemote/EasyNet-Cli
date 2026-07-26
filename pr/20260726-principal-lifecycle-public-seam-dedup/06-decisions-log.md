# Decisions log

2026-07-26:
- `PrincipalLifecycle` is retained because the SPEC/conformance cases explicitly require it.
- `PrincipalProvider` is retired because it duplicates the lifecycle seam and functions only as a second public abstraction for the same capability.
