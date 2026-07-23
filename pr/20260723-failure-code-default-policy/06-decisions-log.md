# Decisions Log

## 2026-07-23

- Treat the unproven-error path as caller-owned default state-machine policy,
  not a compatibility fallback.
- Do not claim bidi `session_failure` behavioral coverage from the current
  filter: it compiles the lib target but matches 0 tests. Dedicated terminality
  coverage remains a separate follow-up item.
