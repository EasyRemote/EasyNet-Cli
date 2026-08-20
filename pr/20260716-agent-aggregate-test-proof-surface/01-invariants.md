# Invariants

- Production Agent aggregate reads remain repository-owned.
- No caller may reopen `agents.json` or `local-agents.json` to bypass
  `AgentAggregateRepository`.
- Runtime behavior, persistence shape, public CLI behavior, and SDK public
  interfaces are unchanged.
- Test-only assertions may inspect proof metadata, but proof-only helpers must
  not expand production API surface.
- Hosted Agent identity lookup by name or URA must keep returning the same
  Agent URA and signing authority used by governance teach authorization.
