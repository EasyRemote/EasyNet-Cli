# Invariants

- `federation.resolve_key` request shape has one Rust owner:
  `federation_wrappers::ResolveKeyRequest`.
- Admission key resolution passes canonical facts to that owner and never
  hand-writes resolve-key JSON objects.
- Empty presented public keys remain absent on the wire; they do not become
  blank fields.
- Existing wire fields remain stable for public behavior.
- Cross-realm key resolution remains fail-closed on encode/decode errors.
