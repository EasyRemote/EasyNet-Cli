# Boundary Proof

## Owner

`access_control.rs` owns the JSON ability adapter boundary. The lower admission
and persistence structs still contain `owner_user_id` and `principal_id` because
the matcher and store index by derived scalar keys internally.

## Invariants

1. Public mutations require `owner_ura` and `actor_ura`.
2. Non-token mutations require `principal_ura`; token mutations derive the
   internal principal key from `token_id`.
3. Nested grant/request payloads cannot supply `owner_user_id` or
   `principal_id`.
4. Persistence structs receive scalar keys only after URA parsing succeeds.

## Non-goals

- Do not change stored RFC-014 grant/request schema.
- Do not change SDK projection DTOs in this slice.
- Do not modify unrelated architecture-convergence gates already staged for a
  different root fork.
