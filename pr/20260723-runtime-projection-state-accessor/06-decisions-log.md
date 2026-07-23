# Decisions Log

## 2026-07-23

- Treat `as_runtime_state` as an internal architecture smell because it exposes
  persistence shape as the caller-facing concept.
- Preserve the persisted `RuntimeState` schema and public CLI behavior; this
  slice is convergence of ownership and naming, not a data migration.
