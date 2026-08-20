# Invariants

- `agent.stop` removes runtime/catalog/authority rows and always preserves the
  registered root directory.
- `agent.purge` is the only public destructive lifecycle ability that may remove
  the exact canonical `root_path` stored in the Agent registry row.
- Catalog metadata must mark only `agent.purge` as destructive among stop/purge
  lifecycle abilities.
- Static descriptors must preserve the same boundary:
  `agent.stop` has `destructive=false`, while `agent.purge` has
  `destructive=true` and requires Manage authority.
- `agent.stop` must reject destructive `purge` input and direct callers to
  `agent.purge`.
