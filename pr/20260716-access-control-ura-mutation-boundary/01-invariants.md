Invariants
==========

1. Access-control mutation ownership is expressed by `owner_ura`.
2. User principals in mutation requests are expressed by `principal_ura`, not
   scalar `principal_id`.
3. `principal_id` may be read from provider projections but is not accepted as
   a mutation fallback.
4. Outgoing `created_grant`, permission request and authority-proof mutation
   payloads do not contain `owner_user_id` or `principal_id`.
5. Go and Python enforce the same fail-closed scalar-input behavior.
6. Existing read-side DTO fields remain source-compatible.
