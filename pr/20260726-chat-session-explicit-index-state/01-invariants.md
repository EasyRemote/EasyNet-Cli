# Invariants

1. Existing `index.json` files must be parsed strictly.
2. Malformed or unreadable existing indexes must fail closed.
3. Missing `index.json` is represented as explicit load state.
4. Fresh-agent reads may project missing state to an empty session index.
5. Mutating session writes may initialize from missing state explicitly.
6. Inventory validation must still reject pointer inconsistency.
7. No storage reader may directly collapse `NotFound` into `SessionIndex::default()`.
