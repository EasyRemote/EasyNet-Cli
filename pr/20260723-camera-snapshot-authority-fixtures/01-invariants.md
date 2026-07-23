# Invariants

1. Camera media abilities remain Device-hosted.
2. Metadata snapshot tests use an explicit Device authority root.
3. Runtime-backed camera execution tests use the same explicit Device authority
   root.
4. Resource subject validation remains envelope-driven.
5. Recording lifecycle tests preserve deterministic session cleanup.
6. No production constructor behavior changes.
7. No fallback identity, compatibility route, or synthetic production signer is
   introduced.
