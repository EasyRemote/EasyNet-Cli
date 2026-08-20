# API Contract

No public CLI or SDK interface changes.

Internal contract:

- `skill.list` must not call `invoke_local_ability`.
- `<user>.api_key.list` must not call `invoke_local_ability`.
- Mutating product abilities remain action invocations until the action issuer
  is split into a named command boundary.
