# Invariants

1. Ability management remains Device-hosted.
2. Metadata-only registration tests use an explicit Device authority root.
3. Runtime execution tests use an explicit Device authority root when attaching
   LocalRuntime.
4. Public ability names and handler behavior remain unchanged.
5. No production constructor behavior changes.
6. No fallback identity, compatibility route, or synthetic production signer is
   introduced.
