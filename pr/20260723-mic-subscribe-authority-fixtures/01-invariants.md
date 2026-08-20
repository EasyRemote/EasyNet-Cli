# Invariants

1. `mic.subscribe` remains Device-hosted.
2. Metadata snapshot tests use an explicit Device authority root.
3. Runtime-backed stream tests use the same explicit Device authority root.
4. Resource subject validation remains descriptor/envelope driven.
5. No production constructor behavior changes.
6. No fallback identity, compatibility route, or synthetic production signer is
   introduced.
