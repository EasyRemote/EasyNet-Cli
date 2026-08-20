# Invariants

1. Ordinary policy admission has one owner authority: canonical trust/URA facts plus verified authority proofs.
2. Local credentials are bootstrap inputs only; they are not ordinary policy facts.
3. A missing trust owner for a device is not repaired by filesystem state.
4. Malformed local credentials must not fail ordinary policy admission; they are outside that state machine.
5. Bootstrap authority remains explicitly bounded by ability, action, subject, caller, and callee.
