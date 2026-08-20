# Invariants

1. User public-key registry reads bind to the runtime-state read subject.
2. User public-key registration remains an explicit mutation path.
3. The reconciliation state machine keeps the same public states:
   `ExistingTrusted`, `ExistingRegistered`, `CreatedRegistered`.
4. Missing or malformed read response remains fail-closed.
5. No fallback to daemon/device subject is introduced.
