# Invariants

1. `skill.list` production code must not call `local_agents::load`.
2. `skill.list` production code must not mention `LocalAgentsFile` or inspect `.hosted_agents`.
3. Hosted Agent display-name and Agent URA lookups for skill scope must pass through an Agent aggregate projection object.
4. `resource_ura` derivation must keep using explicit scoped Agent URA when present, and otherwise derive from the hosted owner projection by local agent name.
5. The ability response remains `{ "items": [...] }` and keeps existing field names.

## Boundary Proof

The ability layer can ask "which local owner name matches this Agent URA?" and "which hosted Agent URA belongs to this local owner name?". It cannot know how hosted Agent identities are stored or loaded.

The aggregate layer can inspect durable hosted identity rows because it is the persistence/domain read boundary for Agent state.
