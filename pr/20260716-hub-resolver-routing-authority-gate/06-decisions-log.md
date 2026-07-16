# Decisions Log

- 2026-07-16: Selected a gate-only slice because the production code already
  models the desired routing authority boundary, but the convergence gate did
  not pin it.
- 2026-07-16: Kept `HubResolver` behavior unchanged and guarded source
  precedence instead. The concrete product use case is remote invocation
  delegation where operator-configured peer routes must remain authoritative and
  directory-observed endpoints must require explicit auto-route opt-in.
