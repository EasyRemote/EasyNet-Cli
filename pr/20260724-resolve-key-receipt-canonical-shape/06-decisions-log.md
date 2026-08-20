# Decisions Log

## 2026-07-24

- Selected `ResolveKeyReceipt` because it retained unused old fields and permissive parsing while current hub dispatch already produces a richer canonical `ResolveKeyResponse`.
- Kept `public_keys_b64` and principal owner projections in the client DTO even though the URA join path only consumes `public_key_hex`; complete canonical parsing is the boundary, not partial field extraction.
