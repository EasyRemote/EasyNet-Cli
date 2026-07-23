# Invariants

1. Chat route registration remains Device-owned.
2. Tests declare Device authority explicitly.
3. `HomeGuard` remains only for HOME-rooted fixture files, not catalog
   authority.
4. Existing chat RPC/stream registration behavior remains unchanged.
5. No compatibility fallback or synthetic production identity is introduced.
