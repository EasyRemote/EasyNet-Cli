# Boundary Proof

## Ownership

Admin + Gateway DTO projection belongs to the daemon SDK profile. Go and Python
facades must project daemon-owned lifecycle and trust facts into the same
canonical SDK DTO fields, not preserve product-era or casing compatibility
names.

## Invariants

1. Go Admin runtime and Python profile bridge output projection accept canonical
   snake_case field names.
2. CamelCase output names are rejected by absence, not silently normalized.
3. Generic `id` and `status` fallbacks do not stand in for typed Admin DTO
   fields such as `token_id`, `credential_id`, `session_id`, or `state`.
4. Request DTO defaults remain typed SDK request data, not legacy output alias
   parsing.
5. No URI terminology or product-specific SDK abstraction is introduced.

## Rejected Designs

- Keeping alternate output aliases for convenience: rejected because provider
  profiles must be latest-only and should expose daemon contract regressions.
- Adding per-field deprecation comments while preserving decoding: rejected
  because the objective requires removing obsolete compatibility paths.
