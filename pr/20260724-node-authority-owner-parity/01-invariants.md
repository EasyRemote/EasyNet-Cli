# Invariants

- Session authority subject ownership is explicit: a session subject owner must match `session_owner_user_id`.
- All-zero principal placeholders are rejected before transport or daemon invocation.
- The SDK model remains product-neutral; fields describe canonical runtime authority facts only.
- Node, Go, and Python must converge on one authority shape instead of language-local subsets.
- The daemon minting request wire remains stable unless the provider contract explicitly supports new fields.
