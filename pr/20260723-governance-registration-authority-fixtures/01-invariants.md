# Invariants

1. Governance registration tests declare a Device authority explicitly.
2. Production ability registration remains unchanged.
3. No test may read host pairing, local key service, or ambient daemon identity
   just to assert registration.
4. Ability names, descriptors, schemas, and public RPC behavior remain
   unchanged.
5. No compatibility fallback or synthetic production identity is introduced.
