# Invariants

- Session-authority subject admission has one Python SDK owner.
- Exact subject equality remains valid.
- User-owned and agent-owned resource admission is derived from parsed URA
  owner components, never substring matching.
- Delegation subject checks remain exact.
- Public SDK behavior is unchanged; only internal helper ownership changes.
- SPEC v2 gate must reject reintroduced session-history exact-match helpers or
  authorized-session-local subject-admission wrappers.
