# Invariants

1. Wrappers are convenience records over governed abilities, not a second
   protocol.
2. File and session records must use owner URAs.
3. Session records require explicit `state`.
4. Runtime Core owns execution, stream, and bidi transport.
5. Backend/product code owns HTTP and WebSocket bridge policy.
6. No non-URA naming and no legacy input aliases are introduced.
