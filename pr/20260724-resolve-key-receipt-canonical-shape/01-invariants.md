# Invariants

## Semantic invariants

- `federation.resolve_key` returns key material facts, not directory status facts.
- `public_key_b64` and `public_key_hex` describe the same resolved key.
- `public_keys_b64` is the canonical multi-key projection; an empty array remains a valid miss/empty-set fact only where the producer emits it.

## Safety invariants

- Unknown receipt fields fail closed.
- Retired directory/status fields cannot be interpreted as key resolution success.
- Principal owner projections remain explicit optional facts, never inferred from old aliases.

## Boundedness invariants

- The join path still performs one descriptor-bound `federation.resolve_key` invocation.
- No fallback parser or second lookup is introduced.
