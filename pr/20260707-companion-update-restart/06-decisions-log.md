# Decisions Log

- 2026-07-07: Model update restart as manager-level `stop` then `start` instead of adding a new platform trait method.
- 2026-07-07: Keep platform supervisors primitive. The manager owns restart orchestration so macOS, Windows, and Linux adapters do not duplicate lifecycle policy.
