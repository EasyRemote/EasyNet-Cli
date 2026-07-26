# Decisions Log

## 2026-07-26

- Selected resolver config strictness because codegraph showed a real
  production serde boundary that accepted unknown fields, while many other
  legacy hits were already fail-closed tests or explicit non-runtime adapters.
- Do not add migration. The user explicitly allowed old data cleanup, and the
  architecture direction is canonical runtime convergence rather than stale
  config preservation.
- Retain valid endpointless FQDN resolution semantics for current config. The
  retired behavior is unknown-field acceptance, not the explicit unresolved
  FQDN state.
