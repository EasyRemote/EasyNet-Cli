# Decisions Log

## 2026-07-16

- Chose test-only confinement rather than `allow(dead_code)` because the issue
  is boundary width, not warning noise.
- Kept repository-level production loaders unchanged because they are the real
  daemon read boundary for governance, skill, and dispatch paths.
- Updated the convergence gate to require the production repository workspace
  resolver instead of requiring the snapshot proof helper to compile in
  production.
- Rejected removing the proof helpers outright because unit tests use them to
  pin aggregate ownership semantics.
