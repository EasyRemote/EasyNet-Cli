# Invariants

1. `meta.list_resources` remains Device-owned.
2. Registration tests use explicit Device authority.
3. Parser and projection tests stay pure and filesystem-independent where
   already pure.
4. Public ability name, schema, description, and wire shape remain unchanged.
5. No fallback identity or compatibility route is introduced.
