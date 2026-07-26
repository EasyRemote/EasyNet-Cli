# API Contract

Inputs remain base64-encoded JSON wire objects under existing metadata keys:

- `x-runtime-delegation`
- `x-runtime-session-authority`

Canonical wire object:

- `payload`: canonical authority payload.
- `signature`: non-empty signature string.

Error behavior:

- Unknown wire or payload fields return `AUTHORITY_FORMAT_INVALID`.
- Missing required fields continue to return `AUTHORITY_FORMAT_INVALID`.
- Expired authority continues to return `AUTHORITY_EXPIRED`.

Tenant and identity rules:

- Authority payloads continue to use canonical URA fields.
- Subject/session owner consistency remains enforced by existing validators.
