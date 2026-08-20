# API Contract

## Request

The public request shape remains:

```json
{
  "callee_ura": "...",
  "ability": "...",
  "call_mode": "...",
  "caller_ura": "...",
  "subject_ura": "..."
}
```

`caller_ura` and `subject_ura` remain accepted for SDK compatibility and tuple
visibility, but descriptor resolution no longer uses them to create a remote
probe.

## Response

On success, the resolver returns the provider catalog row with:

- `descriptor_ref`
- `ability_ura`
- `owner_ura`
- `name`
- `version`
- `descriptor_hash`
- `call_mode`
- `admission_action`
- `source`

## Errors

- Invalid request fields return `INVALID_ARGUMENT`.
- Missing local or realm catalog data returns `DESCRIPTOR_NOT_FOUND`.
- Runtime owner discovery failure remains `CALLER_IDENTITY_UNAVAILABLE`.
- Signer availability is not part of descriptor resolution.

## Tenant Rules

The resolver must not cross tenant boundaries by probing remote owners. Realm
catalog projection is the only accepted cross-owner data source at this layer.
