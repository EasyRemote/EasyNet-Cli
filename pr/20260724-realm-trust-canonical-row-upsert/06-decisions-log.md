# Decisions Log

## 2026-07-24

- Treat existing-row no-op for device trust entries as stale compatibility behavior because it can report successful auto-wire while preserving an obsolete key or role.
- Use one canonical row materialization path for device and hub entries to remove duplicated role-specific update semantics.
- Preserve an existing row's `added_at_unix_ms` during replacement so repeated join auto-wire remains byte-stable while still correcting stale trust facts.
