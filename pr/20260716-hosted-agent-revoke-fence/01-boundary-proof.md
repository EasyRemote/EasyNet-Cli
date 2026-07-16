# Boundary Proof

The Hub durable inventory is the source of truth for hosted Agent publication
and revoke state. Process-local presence and read-model stores are projections
that may be repaired after the durable FSM reaches `Applied`.

The state machine boundary is:

1. register the hosted Agent in one durable inventory slot keyed by logical
   Agent URA and signing authority;
2. prepare a revoke transaction with the exact canonical command and delivery
   fence;
3. apply the prepared command only to the matching authority slot and matching
   generation;
4. remove process-local projections by generation after durable apply;
5. replay `Applied` outcomes without reinterpreting absence as proof.

Addressing, session prelude, and remote invoke code may construct transport
payloads, but they do not own the durable revoke truth.
