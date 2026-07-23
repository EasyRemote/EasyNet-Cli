# Decisions Log

- Preserve public wire field `agent_ura`; this commit changes ownership of
  request construction, not the external ability API.
- Treat `ResolveKeyRequest` in `daemon::federation::wire_contract` as the
  single owner of outbound `federation.resolve_key` request projection.
- Remove `ResolveKeyArgs` from the federation client contract because it was a
  duplicate request DTO outside the canonical federation wire contract.
- Make admission and join callers consume the same DTO so presented-pubkey
  pinning and request byte encoding cannot diverge by call site.
- Extend the canonical runtime convergence v2 gate with a negative fixture so
  raw resolve-key JSON construction cannot be reintroduced as a compatibility
  path.
