# Invariants

1. Node Directory + Identity is a seam: public DTO/client shape exists, but no
   daemon/C ABI provider is introduced.
2. The seam must not parse, build, or canonicalize URAs or DescriptorRefs.
   Identity transport delegates own all grammar projection.
3. Directory list calls are bounded and page-shaped. The facade must not do
   implicit per-agent or per-ability fan-out.
4. The Node facade keeps the same profile vocabulary as Go/Python:
   `directory_identity`, `DirectoryClient`, and `IdentityClient`.
5. The SDK remains URA-only. No URI-era naming, aliases, or compatibility
   fields are added.
