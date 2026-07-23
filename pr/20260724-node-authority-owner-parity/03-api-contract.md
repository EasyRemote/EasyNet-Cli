# API Contract

Public additions:
- `SessionAuthority.sessionOwnerURA`
- `SessionAuthority.creatorPrincipalURA`
- `SessionAuthorityRequest.sessionOwnerURA`
- `SessionAuthorityRequest.creatorPrincipalURA`
- JSON fields `session_owner_ura` and `creator_principal_ura` when present in public object projections.

Validation:
- `session_owner_ura`, when present, must be a canonical user URA and must agree with `session_owner_user_id`.
- `creator_principal_ura`, when present, must be a canonical URA and must agree with `creator_principal_id`.
- All fields must reject all-zero principal placeholders.

Compatibility:
- Existing callers that only pass scalar IDs remain source-compatible.
- No legacy fallback or product-specific default owner is introduced.
