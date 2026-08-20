# Decisions Log

## 2026-07-23

- Chose strict callee extraction rather than a mode flag on the old helper because all current callers are dispatch paths that require a complete invocation target tuple.
