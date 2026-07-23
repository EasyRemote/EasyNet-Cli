# Invariants

1. `agent.list` production registration remains Device-owned.
2. Tests declare a Device authority explicitly.
3. Tests must not read host pairing, key service, or ambient daemon identity.
4. Agent row projection shape remains unchanged.
5. No compatibility fallback or synthetic production identity is introduced.
