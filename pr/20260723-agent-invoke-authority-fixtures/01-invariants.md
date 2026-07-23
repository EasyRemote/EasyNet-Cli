# Invariants

1. `agent.invoke` remains Device-owned.
2. Runtime-backed test catalogs declare Device authority explicitly.
3. The unset dispatch-handle test must still skip `OnceLock::set`.
4. No production constructor behavior changes.
5. No fallback identity, compatibility route, or synthetic production signer is
   introduced.
