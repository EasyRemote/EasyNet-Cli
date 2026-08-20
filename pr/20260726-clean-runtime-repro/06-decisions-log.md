# Decisions Log

## 2026-07-26

- Treat local data cleanup as diagnostic scope only. It can remove stale
  contamination, but it must not become a compatibility mechanism.
- Use isolated HOME/Docker reproduction before code changes so that fixes target
  canonical runtime behavior rather than a stale user-machine artifact.
- Keep the Docker E2E aligned with PrincipalLifecycle proof custody by supplying
  an explicit one-time bootstrap proof reference.
- Replace Hub-side `device list` readiness with device-owned canonical status
  readiness. Hub authority must not pretend to be a user-scoped federation
  discovery client.
