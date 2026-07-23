# Decisions Log

- Decision: do not move product mutations in this slice.
  Rationale: install, upgrade, remove, create, and revoke mutate daemon-owned
  state and need the action invocation path until a dedicated named action
  issuer is introduced.
