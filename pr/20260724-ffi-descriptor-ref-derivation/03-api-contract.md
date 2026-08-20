# API contract

No public API shape changes.

Descriptor catalog entry fields remain:

- `name`
- `owner_ura`
- `ability_ura`
- `descriptor_ref`
- `version`
- `descriptor_hash`
- `call_mode`
- `admission_action`

Failure semantics:

- Invalid descriptor identity fails catalog construction.
- Missing descriptor remains `DESCRIPTOR_NOT_FOUND` through the existing runtime catalog miss path.
- No route/remote fallback is introduced.
