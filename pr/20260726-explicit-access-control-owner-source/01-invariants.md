# Invariants

- Access-control ownership must be declared, not inferred from subject/callee compatibility defaults.
- SDK clients must build the same canonical argument set across Go and Python.
- Missing `owner_source` is an invalid request, not a policy denial and not a fallback to subject ownership.
- No storage key such as `owner_user_id` or `principal_id` may cross the public SDK/ability boundary.
