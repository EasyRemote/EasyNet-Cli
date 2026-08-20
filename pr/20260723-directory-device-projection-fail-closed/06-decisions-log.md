# Decisions Log

## 2026-07-23

- Chose fail-closed projection instead of filtering invalid presence rows silently because silent filtering would hide registry corruption.
- Kept the directory wire vocabulary unchanged; this refactor changes validation and state application ownership, not the event names.
