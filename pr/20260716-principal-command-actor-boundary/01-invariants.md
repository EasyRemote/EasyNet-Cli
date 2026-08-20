# Invariants

- `actor_ura` in a PrincipalLifecycle command is always an explicit command
  field before daemon dispatch.
- A missing CLI `--actor-ura` may map to subject-self authorization only through
  a named CLI facade state, never inside the JSON serializer.
- The daemon remains the authority for canonical URA validation and state
  transition admission.
- Command idempotency, proof kind, proof reference, and expected version are
  unchanged by actor-source refactoring.
- No scalar account, user id, or private key fields are introduced.
