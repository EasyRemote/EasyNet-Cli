# Execution Checklist

- [x] Add constructor and byte encoder to `ResolveKeyRequest`.
- [x] Migrate `FederatedKeyResolver` away from raw `serde_json::json!`.
- [x] Migrate `join` away from the duplicate federation client DTO.
- [x] Remove legacy-shape commentary from admission resolver.
- [x] Add v2 gate coverage preventing raw resolve-key JSON construction in admission and join.
- [x] Run targeted tests, gates, codegraph, and rg verification.
