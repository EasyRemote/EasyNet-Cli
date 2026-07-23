# Invariants

1. Agent lifecycle production registration behavior remains unchanged.
2. The registration smoke test stays metadata-only; no runtime is introduced.
3. The test catalog uses an explicit Device authority root.
4. No fallback identity, compatibility admission route, or synthetic production
   signer is introduced.
5. Lifecycle state-machine tests and handler tests remain unchanged.
