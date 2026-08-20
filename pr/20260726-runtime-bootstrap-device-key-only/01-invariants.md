# Invariants

1. A bootstrap key proves exactly one runtime identity: the canonical Device URA for `(realm, node_id)`.
2. A Device bootstrap proof must not authorize an Agent URA.
3. `owner_id` is a partitioning fact for node ownership, not an identity alias constructor.
4. Key resolver misses for Agent aliases must fail closed.
5. The public ability remains descriptor-bound and does not gain a CLI-side ack path.
