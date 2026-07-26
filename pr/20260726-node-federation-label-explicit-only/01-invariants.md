# Invariants

- Product UI labels must be explicitly human-readable facts.
- Stable runtime ids remain machine identifiers and must not be promoted into
  operator-facing display names.
- Local nodes and federated nodes without explicit labels render the same
  absence state: no `via` suffix.
- Node projection remains shared by all CLI callers; no command may reimplement
  federation label fallback logic.
