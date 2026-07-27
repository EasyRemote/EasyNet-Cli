## Invariants

- Ability call mode is part of the governed descriptor identity and route selection.
- The daemon must not infer RPC mode from absence of metadata.
- Declarative plugins remain RPC-only by explicit manifest declaration, not by parser default.
- Sidecar stream/bidi/rpc routing remains selected only from manifest-declared `call_mode`.
- Missing call mode is invalid input, not a compatibility case.
