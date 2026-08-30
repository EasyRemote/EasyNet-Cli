# Invariants

1. The paired realm trust anchor is authoritative for the current realm
   Authority key.
2. The presented signing key must match the exact anchored Authority key.
3. Same-session key resolution must never be part of inbound dispatch.
4. Missing or stale local Authority trust fails closed; it is not repaired by
   a compatibility fallback.
5. Cross-realm User and Authority callers remain ephemeral Hub-attested
   identities and are not persisted into the destination trust anchor.
