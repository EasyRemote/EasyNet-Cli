# Invariants

1. Existing `local-agents.json` files parse strictly.
2. Malformed or unreadable existing files fail closed.
3. Missing storage is represented as explicit load state.
4. First-boot empty projection is owned by an explicit projection helper.
5. Agent lifecycle and aggregate read paths use the explicit projection helper.
6. The stable public `load()` shape remains a read projection.
7. No storage reader directly checks path existence and returns
   `LocalAgentsFile::default()`.
