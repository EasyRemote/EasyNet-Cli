# Invariants

1. The `ABILITIES` table remains the single source of truth for media metadata.
2. Media metadata tests use an explicit Device authority root.
3. Hub voice seams remain descriptor metadata only and are not published as
   unavailable handlers.
4. Real media handler slots remain excluded from stub registration.
5. No production constructor behavior changes.
6. No fallback identity, compatibility route, or synthetic production signer is
   introduced.
