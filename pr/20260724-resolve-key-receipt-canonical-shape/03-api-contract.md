# API Contract

## Response

Canonical resolve-key receipt fields:

- `public_key_b64`
- `public_key_hex`
- `public_keys_b64`
- optional `principal_owner_ura`
- optional `principal_owner_user_id`
- optional `principal_owner_username`

Retired fields:

- `agent_ura`
- `status`
- `key_id`
- `rotation_epoch`

## Errors

Retired fields must parse as unknown fields.

## Tenant and identity rules

No change. Caller/callee/subject binding remains enforced by invocation admission.
