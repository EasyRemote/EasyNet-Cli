# Invariants

1. Runtime descriptor catalogue reads are authority-scoped runtime reads, not target-owned product operations.
2. `provider: "ability_descriptor"` may resolve only runtime catalogue abilities such as `meta.list_abilities` and `meta.list_resources`.
3. The catalogue read subject is the realm authority URA for the callee realm.
4. A device subject must never be accepted for ability descriptor catalogue resolution.
5. A user-owned runtime-state read subject is reserved for receipt-history reads and must not authorize ability descriptor catalogue reads.
6. Invalid subject facts fail at the descriptor resolver boundary before remote route resolution, caller-signer lookup, or admission.
7. The provider kind owns provider-specific request validation; callers do not duplicate provider policy.
