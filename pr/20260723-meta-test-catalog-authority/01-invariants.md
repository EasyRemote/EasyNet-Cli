# Invariants

1. Production catalogue constructors remain fail-closed on missing local
   authority.
2. Metadata tests use explicit authority roots.
3. Live-runtime metadata tests continue to attach a canonical `LocalRuntime`.
4. No fallback Device URA or synthetic production identity is introduced.
5. Public CLI, SDK, daemon, and descriptor behavior remain unchanged.
