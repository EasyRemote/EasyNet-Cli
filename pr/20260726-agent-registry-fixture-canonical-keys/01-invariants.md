# Invariants

1. The durable agent registry key is canonical `AgentId`, not a bare product
   display name.
2. Fixture data must express the same owner model as production data.
3. Public command names remain short selectors where the CLI contract expects
   short names.
4. No compatibility migration from bare names is reintroduced.
5. A fixture may derive `default/<name>` for registry lookup, but must not make
   production persistence accept bare names.
