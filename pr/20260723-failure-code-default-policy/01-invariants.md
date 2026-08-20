# Invariants

## Semantic invariants

- Specific codes are emitted only when extracted from a current reason token or
  recognized current phrase.
- If no specific code is proven, the caller's default state-machine code is
  emitted unchanged after normalization.
- Explicit codes still win over reason-derived codes.

## Safety invariants

- The classifier must not invent product-specific failure codes.
- The default code remains caller-owned state-machine policy.
- Stage/security classification remains derived from the final code.

## Boundedness invariants

- Classification remains pure and allocation-bounded by input length.
- No I/O, global state, or runtime dependency is introduced.
