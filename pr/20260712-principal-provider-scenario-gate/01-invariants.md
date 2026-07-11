# Invariants

1. No Backend/OAuth/HTTP account field participates in the scenario.
2. User creation after the first principal must be authorized by durable
   enrollment or grant proof, not by a bare proof kind.
3. Multiple public keys for one Principal must remain distinct lifecycle facts.
4. Rotation and revocation must update both lifecycle state and RuntimeTrust
   public-key admission state.
5. Recovery must add a key through the configured recovery policy without
   replacing sibling keys.
6. Recovery from a suspended principal is an explicit state transition back to
   Active when backed by an enabled, unconsumed recovery policy.
7. Deleted principal state is terminal: even a matching unconsumed recovery
   policy must not recreate admission state or register a replacement key.
8. Suspended and deleted states must persist after reload.
9. The scenario must not claim final cutover until a real daemon/CLI/URA join
   E2E exists.
