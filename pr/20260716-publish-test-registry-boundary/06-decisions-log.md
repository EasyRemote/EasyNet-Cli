# Decisions Log

## 2026-07-16

- Chose this slice after full cutover readiness passed but repeatedly surfaced
  a production-scope unused `agent_registry` import.
- Kept direct registry writes in tests because those helpers materialise
  `HomeGuard`-isolated fixtures and do not define runtime ownership.
- Did not address other warnings in this slice because generated route
  constants and unrelated dirty modules need separate owner decisions.
