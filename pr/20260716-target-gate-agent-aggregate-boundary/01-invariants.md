# Invariants

- Admission target locality must be decided from one coherent Agent aggregate snapshot per gate construction.
- A hosted Agent URA is local only when the full `(realm, user_id, agent_id)` tuple is present in the hosted identity projection.
- The credential-plus-registry path remains bounded to the credential `(realm, user_id)` and an exact durable Agent ID.
- Aggregate load failure must not create an admission bypass. The gate may still match daemon, hub, and device URAs, but Agent URA locality fails closed.
- The aggregate repository remains the only owner that pairs durable Agent registry and hosted identity projection reads.
