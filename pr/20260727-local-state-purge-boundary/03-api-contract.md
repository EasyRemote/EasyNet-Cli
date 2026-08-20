# API contract

## Request

`easynet reset [--force] [--yes] [--purge-local-state]`

## Response

- Without `--purge-local-state`, reset removes pairing credentials as before.
- With `--purge-local-state`, reset removes the local EasyNet state directory
  after the same runtime and confirmation gates.

## Error contract

- Running runtime without `--force` remains an error.
- Malformed lifecycle projection remains an error before deletion.
- Local filesystem deletion errors are surfaced; no best-effort partial success
  is reported as clean.

## Tenant rules

The command acts only on the current user's local EasyNet state root.
