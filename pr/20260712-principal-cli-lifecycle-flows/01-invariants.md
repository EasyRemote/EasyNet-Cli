# Invariants

1. CLI commands are facade-only. Every lifecycle mutation is submitted to a
   `principal.lifecycle.*` daemon ability with explicit command, proof and
   optimistic version fields.
2. CLI must not verify lifecycle proofs, persist lifecycle truth or infer
   Backend account state.
3. Replacement/add/recovery keys may be daemon-managed local keys or explicit
   public-key projections. Private key material never leaves daemon key-service.
4. CLI request lowering must stay aligned with Go/Python SDK PrincipalLifecycle
   request shapes.
5. State transitions remain provider-owned: CLI does not decide whether a
   principal is active, suspended, deleted, recoverable or authorized.
6. Grants remain generic lifecycle authority facts, not product roles.
7. CLI failure output must surface daemon PrincipalLifecycle denial reasons for
   recovery replay and deleted-principal terminality; it must not convert them
   into local fallback or generic account-authentication behavior.
8. This slice does not claim standalone-Hub cutover. E2E join, multi-user,
   recovery and restart persistence gates remain required.
