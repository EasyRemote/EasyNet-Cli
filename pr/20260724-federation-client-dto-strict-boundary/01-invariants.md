# Invariants

## Semantic invariants

- A federation receipt shape has one canonical schema at the client boundary.
- Unknown receipt fields are protocol errors, not optional extension points.
- Canonical URA fields must remain named as URA, never URI.

## Safety invariants

- Receipt parsing must not permit product-specific fields to smuggle authority, account, or directory state.
- Resolve rows must not accept retired agent identity aliases when the canonical field is `ura`.
- Enrollment proof references must not accept caller/user/device product account aliases.

## Boundedness invariants

- Parsing failure happens synchronously at the DTO boundary.
- No fallback resolver or alternate receipt canonicalizer is introduced.
