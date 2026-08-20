# Access-Control Actor URA Gate

## Objective

Close the A88 regression surface with an executable architecture rule. Access-control mutations must persist audited actor identity only from a canonical `actor_ura`, never from scalar `owner_user_id` or nested DTO compatibility fields.

## Invariants

1. `authority.binding.revoke` has a typed request boundary with `deny_unknown_fields`.
2. The revoke request boundary exposes `owner_ura` and `actor_ura`, not `owner_user_id` or `actor_user_id`.
3. A missing `actor_ura` fails before store mutation.
4. A scalar actor identifier fails canonical URA parsing before audit persistence.
5. Revoke persists the validated `actor_ura` into the access-control store.

## Effect

This slice does not change public behavior. It captures the current URA-only access-control mutation boundary in CI so future edits cannot reintroduce scalar identity fallback into revoke audit records.
