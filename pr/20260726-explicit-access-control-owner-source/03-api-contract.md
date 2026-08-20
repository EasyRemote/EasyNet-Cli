# API Contract

## Request

`authority_binding.check` requires:

- `owner_ura`
- `owner_source`
- `caller_ura`
- `principal_kind`
- `callee_ura`
- `subject_ura`
- `ability_ura`
- `action`

`principal_ura` or `token_id` remains required according to principal kind.

## Error

Missing `owner_source` returns an invalid request error containing `owner_source is required`.

## Tenant rule

The owner remains a canonical User URA. The source of that owner resolution is explicitly supplied as runtime policy fact and is not derived from product naming.
