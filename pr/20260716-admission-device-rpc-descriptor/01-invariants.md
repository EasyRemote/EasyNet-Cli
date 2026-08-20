# Invariants

1. RPC admission binds only an RPC descriptor.
2. `session.open` has no RPC compatibility descriptor.
3. A signed Device RPC reaches policy evaluation and writes one replay entry.
4. Repeating the same signed nonce is rejected.
